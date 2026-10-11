#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

// Full MCP in-process transport round trips for the tools still uncovered after
// mcp_integration.rs and tool_lifecycle_integration.rs: get_server_info,
// batch_get_entities, get_latest_version, get_supersession_chain,
// get_incoming_references, get_outgoing_references, get_impact,
// retarget_references (dry_run), get_slice_projection,
// get_storyboard_projection, get_event_model_projection, extract_subgraph,
// diff_entities, put_entity (binpb variant), batch_mutate (non-json
// variant), list_validation_rules, validate_project, delete_by_query,
// list_entity_kinds, list_entities_by_domain, infer_data_flow,
// check_information_completeness.
//
// Same duplex-pipe harness as tool_lifecycle_integration.rs: a fresh
// server/client pair against a fresh in-process gRPC backend, driven by
// rmcp::serve_client over tokio::io::duplex.
//
// search_entities is intentionally skipped: it depends on the search index
// build, which needs more setup than the other tools justify here.

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
    let mcp_server = support::mcp_server(&grpc_endpoint, None);
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

fn args_obj(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    m
}

fn call_tool_result_text(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    support::structured(result)
}

fn base64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn event_entity(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn command_entity(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Command(pb::Command {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn command_slice_entity(
    id: pb::Id,
    title: &str,
    command_id: pb::Id,
    emitted_event_id: pb::Id,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id),
            title: title.into(),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(command_id),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(emitted_event_id),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
    }
}

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn event_model_entity(
    ns: &str,
    slug: &str,
    version: u64,
    title: &str,
    members: Vec<pb::EntityRef>,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            members,
            ..Default::default()
        })),
    }
}

fn entity_ref(kind: pb::EntityKind, ns: &str, slug: &str, version: u64) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version,
        }),
    }
}

fn put_req(entity: pb::Entity, create_only: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        create_only,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    }
}

// get_server_info: static handshake-style tool, no arguments.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_get_server_info_returns_json() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_server_info")
                .with_arguments(args_obj(&[])),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let info: pb::GetServerInfoResponse = support::structured_as(&result, "GetServerInfoResponse");
    assert!(info.features.is_some(), "{info:?}");

    mcp_service.cancel().await.unwrap();
}

// list_entity_kinds / list_validation_rules: static catalog tools, no
// namespace state required.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_static_catalog_tools() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    // Older prompts still pass the retired `json` flag; it is accepted and
    // changes nothing, since every result is JSON now.
    let kinds_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("list_entity_kinds")
                .with_arguments(args_obj(&[("json", serde_json::json!(true))])),
        )
        .await
        .unwrap();
    assert!(!kinds_result.is_error.unwrap_or(false), "{kinds_result:?}");
    let kinds_report = call_tool_result_text(&kinds_result);
    assert!(
        !kinds_report["kinds"].as_array().unwrap().is_empty(),
        "{kinds_report:?}"
    );

    let rules_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("list_validation_rules")
                .with_arguments(args_obj(&[])),
        )
        .await
        .unwrap();
    assert!(!rules_result.is_error.unwrap_or(false), "{rules_result:?}");
    let rules_report = call_tool_result_text(&rules_result);
    assert!(
        !rules_report["rules"].as_array().unwrap().is_empty(),
        "{rules_report:?}"
    );

    mcp_service.cancel().await.unwrap();
}

// Seed one small graph (command -> command_slice -> event, wrapped in an
// event_model) and exercise every read-only graph/analysis tool against it
// in one lifecycle, as encouraged by the mission brief.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_read_tool_lifecycle_over_seeded_graph() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;
    let mut grpc = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();

    let ns = "mcp-gap-graph";
    grpc.put_entity(put_req(event_entity(ns, "placed", 1, "Placed"), true))
        .await
        .unwrap();
    grpc.put_entity(put_req(command_entity(ns, "place", 1, "Place"), true))
        .await
        .unwrap();
    grpc.put_entity(put_req(
        command_slice_entity(
            id(ns, "place-order", 1),
            "Place order",
            id(ns, "place", 1),
            id(ns, "placed", 1),
        ),
        true,
    ))
    .await
    .unwrap();
    grpc.put_entity(put_req(
        event_model_entity(
            ns,
            "model",
            1,
            "Model",
            vec![
                entity_ref(pb::EntityKind::Event, ns, "placed", 1),
                entity_ref(pb::EntityKind::Command, ns, "place", 1),
                entity_ref(pb::EntityKind::CommandSlice, ns, "place-order", 1),
            ],
        ),
        true,
    ))
    .await
    .unwrap();
    // A second version of the command, to give get_latest_version and
    // get_supersession_chain something to walk.
    grpc.put_entity(put_req(command_entity(ns, "place", 2, "Place v2"), true))
        .await
        .unwrap();

    // get_latest_version: no version in params, resolves to the highest.
    let latest_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_latest_version").with_arguments(args_obj(
                &[
                    ("kind", serde_json::json!("command")),
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("place")),
                ],
            )),
        )
        .await
        .unwrap();
    assert!(
        !latest_result.is_error.unwrap_or(false),
        "{latest_result:?}"
    );
    let latest_decoded: pb::GetLatestVersionResponse =
        support::structured_as(&latest_result, "GetLatestVersionResponse");
    let Some(pb::entity::Kind::Command(cmd)) = latest_decoded.entity.unwrap().kind else {
        panic!("expected command");
    };
    assert_eq!(cmd.id.as_ref().unwrap().version, 2, "{cmd:?}");

    // get_supersession_chain: ancestors direction from v2 finds v1.
    let chain_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_supersession_chain").with_arguments(
                args_obj(&[
                    ("kind", serde_json::json!("command")),
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("place")),
                    ("version", serde_json::json!(2)),
                    ("include_descendants", serde_json::json!(false)),
                ]),
            ),
        )
        .await
        .unwrap();
    assert!(!chain_result.is_error.unwrap_or(false), "{chain_result:?}");
    support::structured_as::<pb::GetSupersessionChainResponse>(
        &chain_result,
        "GetSupersessionChainResponse",
    );

    // get_outgoing_references: place-order slice references the command and
    // the event.
    let outgoing_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_outgoing_references").with_arguments(
                args_obj(&[
                    ("kind", serde_json::json!("command_slice")),
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("place-order")),
                    ("version", serde_json::json!(1)),
                ]),
            ),
        )
        .await
        .unwrap();
    assert!(
        !outgoing_result.is_error.unwrap_or(false),
        "{outgoing_result:?}"
    );
    let outgoing_decoded: pb::GetReferencesResponse =
        support::structured_as(&outgoing_result, "GetReferencesResponse");
    assert!(
        !outgoing_decoded.references.is_empty(),
        "{outgoing_decoded:?}"
    );

    // get_incoming_references: the "placed" event is referenced by the slice.
    let incoming_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_incoming_references").with_arguments(
                args_obj(&[
                    ("kind", serde_json::json!("event")),
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("placed")),
                    ("version", serde_json::json!(1)),
                ]),
            ),
        )
        .await
        .unwrap();
    assert!(
        !incoming_result.is_error.unwrap_or(false),
        "{incoming_result:?}"
    );
    let incoming_decoded: pb::GetReferencesResponse =
        support::structured_as(&incoming_result, "GetReferencesResponse");
    assert!(
        !incoming_decoded.references.is_empty(),
        "{incoming_decoded:?}"
    );

    // get_impact: change to the event ripples to whatever references it.
    let impact_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_impact").with_arguments(args_obj(&[
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!(ns)),
                ("slug", serde_json::json!("placed")),
                ("version", serde_json::json!(1)),
                ("max_depth", serde_json::json!(5)),
            ])),
        )
        .await
        .unwrap();
    assert!(
        !impact_result.is_error.unwrap_or(false),
        "{impact_result:?}"
    );
    support::structured_as::<pb::GetImpactResponse>(&impact_result, "GetImpactResponse");

    // batch_get_entities: fetch all three seeded entities in one call.
    let batch_get_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("batch_get_entities").with_arguments(args_obj(&[(
                "refs",
                serde_json::json!([
                    {"kind": "event", "namespace": ns, "slug": "placed", "version": 1},
                    {"kind": "command", "namespace": ns, "slug": "place", "version": 2},
                    {"kind": "command_slice", "namespace": ns, "slug": "place-order", "version": 1},
                ]),
            )])),
        )
        .await
        .unwrap();
    assert!(
        !batch_get_result.is_error.unwrap_or(false),
        "{batch_get_result:?}"
    );
    let batch_get_decoded: pb::BatchGetEntitiesResponse =
        support::structured_as(&batch_get_result, "BatchGetEntitiesResponse");
    assert_eq!(batch_get_decoded.entities.len(), 3, "{batch_get_decoded:?}");

    // get_slice_projection: renders the command_slice projection.
    let slice_proj_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_slice_projection").with_arguments(
                args_obj(&[
                    ("kind", serde_json::json!("command_slice")),
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("place-order")),
                    ("version", serde_json::json!(1)),
                ]),
            ),
        )
        .await
        .unwrap();
    assert!(
        !slice_proj_result.is_error.unwrap_or(false),
        "{slice_proj_result:?}"
    );
    support::structured_as::<pb::GetSliceProjectionResponse>(
        &slice_proj_result,
        "GetSliceProjectionResponse",
    );

    // get_event_model_projection: renders the whole model's projection.
    let em_proj_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_event_model_projection").with_arguments(
                args_obj(&[
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("model")),
                    ("version", serde_json::json!(1)),
                    ("include_cross_model", serde_json::json!(false)),
                ]),
            ),
        )
        .await
        .unwrap();
    assert!(
        !em_proj_result.is_error.unwrap_or(false),
        "{em_proj_result:?}"
    );
    let em_proj_decoded: pb::GetEventModelProjectionResponse =
        support::structured_as(&em_proj_result, "GetEventModelProjectionResponse");
    let _ = em_proj_decoded;

    // extract_subgraph: root at the event_model, closure includes all
    // members.
    let subgraph_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("extract_subgraph").with_arguments(args_obj(&[(
                "root",
                serde_json::json!({"kind": "event_model", "namespace": ns, "slug": "model", "version": 1}),
            )])),
        )
        .await
        .unwrap();
    assert!(
        !subgraph_result.is_error.unwrap_or(false),
        "{subgraph_result:?}"
    );
    support::structured_as::<pb::ExtractSubgraphResponse>(
        &subgraph_result,
        "ExtractSubgraphResponse",
    );

    // diff_entities: the two command versions differ in title.
    let diff_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("diff_entities").with_arguments(args_obj(&[
                (
                    "a",
                    serde_json::json!({"kind": "command", "namespace": ns, "slug": "place", "version": 1}),
                ),
                (
                    "b",
                    serde_json::json!({"kind": "command", "namespace": ns, "slug": "place", "version": 2}),
                ),
            ])),
        )
        .await
        .unwrap();
    assert!(!diff_result.is_error.unwrap_or(false), "{diff_result:?}");
    support::structured_as::<pb::DiffEntitiesResponse>(&diff_result, "DiffEntitiesResponse");

    // infer_data_flow / check_information_completeness: analysis tools over
    // the command_slice scope.
    let flow_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("infer_data_flow").with_arguments(args_obj(&[
                ("kind", serde_json::json!("command_slice")),
                ("namespace", serde_json::json!(ns)),
                ("slug", serde_json::json!("place-order")),
                ("version", serde_json::json!(1)),
            ])),
        )
        .await
        .unwrap();
    assert!(!flow_result.is_error.unwrap_or(false), "{flow_result:?}");
    support::structured_as::<pb::InferDataFlowResponse>(&flow_result, "InferDataFlowResponse");

    let completeness_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("check_information_completeness")
                .with_arguments(args_obj(&[
                    ("kind", serde_json::json!("command_slice")),
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("place-order")),
                    ("version", serde_json::json!(1)),
                ])),
        )
        .await
        .unwrap();
    assert!(
        !completeness_result.is_error.unwrap_or(false),
        "{completeness_result:?}"
    );
    support::structured_as::<pb::CheckInformationCompletenessResponse>(
        &completeness_result,
        "CheckInformationCompletenessResponse",
    );

    // list_entities_by_domain requires a domain -> subdomain -> bounded
    // context -> event_model realizes chain; without one seeded, the tool
    // still round-trips successfully with an empty result set.
    let by_domain_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("list_entities_by_domain").with_arguments(
                args_obj(&[
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("no-such-domain")),
                    ("version", serde_json::json!(1)),
                ]),
            ),
        )
        .await
        .unwrap();
    assert!(
        !by_domain_result.is_error.unwrap_or(false),
        "{by_domain_result:?}"
    );
    let by_domain_report = call_tool_result_text(&by_domain_result);
    assert!(
        by_domain_report
            .get("entities")
            .is_none_or(|e| e.as_array().is_none_or(Vec::is_empty)),
        "{by_domain_report:?}"
    );

    mcp_service.cancel().await.unwrap();
}

// retarget_references: dry_run=true reports the rewrite without persisting
// it, per the mission's explicit request to cover the dry_run branch.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_retarget_references_dry_run_does_not_persist() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;
    let mut grpc = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();

    let ns = "mcp-gap-retarget";
    grpc.put_entity(put_req(event_entity(ns, "old-event", 1, "Old event"), true))
        .await
        .unwrap();
    grpc.put_entity(put_req(event_entity(ns, "new-event", 1, "New event"), true))
        .await
        .unwrap();
    grpc.put_entity(put_req(command_entity(ns, "place", 1, "Place"), true))
        .await
        .unwrap();
    grpc.put_entity(put_req(
        command_slice_entity(
            id(ns, "place-order", 1),
            "Place order",
            id(ns, "place", 1),
            id(ns, "old-event", 1),
        ),
        true,
    ))
    .await
    .unwrap();

    let dry_run_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("retarget_references").with_arguments(
                args_obj(&[
                    ("from_kind", serde_json::json!("event")),
                    ("from_namespace", serde_json::json!(ns)),
                    ("from_slug", serde_json::json!("old-event")),
                    ("from_version", serde_json::json!(1)),
                    ("to_slug", serde_json::json!("new-event")),
                    ("to_version", serde_json::json!(1)),
                    ("dry_run", serde_json::json!(true)),
                ]),
            ),
        )
        .await
        .unwrap();
    assert!(
        !dry_run_result.is_error.unwrap_or(false),
        "{dry_run_result:?}"
    );
    let dry_run_decoded: pb::RetargetReferencesResponse =
        support::structured_as(&dry_run_result, "RetargetReferencesResponse");
    assert_eq!(dry_run_decoded.rewritten, 1, "{dry_run_decoded:?}");

    // dry_run must not have persisted: outgoing refs of the slice still
    // point at the old event.
    let outgoing_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_outgoing_references").with_arguments(
                args_obj(&[
                    ("kind", serde_json::json!("command_slice")),
                    ("namespace", serde_json::json!(ns)),
                    ("slug", serde_json::json!("place-order")),
                    ("version", serde_json::json!(1)),
                ]),
            ),
        )
        .await
        .unwrap();
    let outgoing_decoded: pb::GetReferencesResponse =
        support::structured_as(&outgoing_result, "GetReferencesResponse");
    assert!(
        outgoing_decoded.references.iter().any(|r| r
            .to
            .as_ref()
            .and_then(|t| t.id.as_ref())
            .map(|id| id.slug.as_str())
            == Some("old-event")),
        "dry_run must not persist: {outgoing_decoded:?}"
    );

    mcp_service.cancel().await.unwrap();
}

// put_entity: the raw binpb variant (as opposed to put_entity_json).
#[tokio::test(flavor = "multi_thread")]
async fn mcp_put_entity_binpb_variant() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let entity = event_entity("mcp-gap-put-binpb", "created", 1, "Created via binpb");
    let entity_b64 = base64_encode(&entity.encode_to_vec());

    let put_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity").with_arguments(args_obj(&[
                ("entity_b64", serde_json::json!(entity_b64)),
                ("create_only", serde_json::json!(true)),
            ])),
        )
        .await
        .unwrap();
    assert!(!put_result.is_error.unwrap_or(false), "{put_result:?}");
    let put_decoded: pb::PutEntityResponse =
        support::structured_as(&put_result, "PutEntityResponse");
    assert!(!put_decoded.no_op, "{put_decoded:?}");

    // A second create_only put of the identical entity is a no_op (force
    // defaults to false).
    let put_again_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity").with_arguments(args_obj(&[
                ("entity_b64", serde_json::json!(entity_b64)),
                ("create_only", serde_json::json!(false)),
            ])),
        )
        .await
        .unwrap();
    assert!(
        !put_again_result.is_error.unwrap_or(false),
        "{put_again_result:?}"
    );
    let put_again_decoded: pb::PutEntityResponse =
        support::structured_as(&put_again_result, "PutEntityResponse");
    assert!(put_again_decoded.no_op, "{put_again_decoded:?}");

    // get_entity with format=binpb hands back the protobuf the binpb write
    // path edits and passes to put_entity.
    let get_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity").with_arguments(args_obj(&[
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!("mcp-gap-put-binpb")),
                ("slug", serde_json::json!("created")),
                ("version", serde_json::json!(1)),
                ("format", serde_json::json!("binpb")),
            ])),
        )
        .await
        .unwrap();
    let get_body = call_tool_result_text(&get_result);
    let raw = base64::engine::general_purpose::STANDARD
        .decode(get_body["binpb_base64"].as_str().unwrap())
        .unwrap();
    let got = pb::GetEntityResponse::decode(raw.as_slice()).unwrap();
    assert!(!got.etag.is_empty(), "{got:?}");
    assert_eq!(got.entity.and_then(|e| e.kind), entity.kind);

    mcp_service.cancel().await.unwrap();
}

// batch_mutate: the non-json variant (EntitySpec-shaped delete op, binpb put
// op) as opposed to batch_mutate_json.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_batch_mutate_binpb_variant() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;
    let mut grpc = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();

    let ns = "mcp-gap-batch-binpb";
    grpc.put_entity(put_req(event_entity(ns, "to-delete", 1, "To delete"), true))
        .await
        .unwrap();

    let new_entity = event_entity(ns, "to-create", 1, "To create");
    let new_entity_b64 = base64_encode(&new_entity.encode_to_vec());

    let batch_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("batch_mutate").with_arguments(args_obj(&[(
                "ops",
                serde_json::json!([
                    {"op": "put", "entity_b64": new_entity_b64, "create_only": true},
                    {"op": "delete", "kind": "event", "namespace": ns, "slug": "to-delete", "version": 1},
                ]),
            )])),
        )
        .await
        .unwrap();
    assert!(!batch_result.is_error.unwrap_or(false), "{batch_result:?}");
    let batch_decoded: pb::BatchMutateResponse =
        support::structured_as(&batch_result, "BatchMutateResponse");
    assert_eq!(batch_decoded.results.len(), 2, "{batch_decoded:?}");

    // The delete op requires kind/namespace/slug/version all present; a
    // malformed delete op (missing version) is rejected as invalid_params
    // before any RPC.
    let malformed_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("batch_mutate").with_arguments(args_obj(&[(
                "ops",
                serde_json::json!([
                    {"op": "delete", "kind": "event", "namespace": ns, "slug": "to-create"},
                ]),
            )])),
        )
        .await
        .unwrap_err();
    match malformed_result {
        rmcp::ServiceError::McpError(e) => {
            assert_eq!(e.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

// validate_project: scope by project_namespace+project_slug; scope by
// domain_namespace+domain_slug; and the invalid_params rejection when
// neither scope is set.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_validate_project_scope_variants() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;
    let mut grpc = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();

    let ns = "mcp-gap-validate-project";
    grpc.put_entity(put_req(
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Project(pb::Project {
                id: Some(pb::Id {
                    namespace: ns.into(),
                    slug: "project".into(),
                    version: 1,
                }),
                title: "Project".into(),
                ..Default::default()
            })),
        },
        true,
    ))
    .await
    .unwrap();
    grpc.put_entity(put_req(
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Domain(pb::Domain {
                id: Some(pb::Id {
                    namespace: ns.into(),
                    slug: "domain".into(),
                    version: 1,
                }),
                title: "Domain".into(),
                ..Default::default()
            })),
        },
        true,
    ))
    .await
    .unwrap();

    let by_project_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("validate_project").with_arguments(args_obj(
                &[
                    ("project_namespace", serde_json::json!(ns)),
                    ("project_slug", serde_json::json!("project")),
                    ("project_version", serde_json::json!(1)),
                ],
            )),
        )
        .await
        .unwrap();
    assert!(
        !by_project_result.is_error.unwrap_or(false),
        "{by_project_result:?}"
    );

    let by_domain_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("validate_project").with_arguments(args_obj(
                &[
                    ("domain_namespace", serde_json::json!(ns)),
                    ("domain_slug", serde_json::json!("domain")),
                    ("domain_version", serde_json::json!(1)),
                ],
            )),
        )
        .await
        .unwrap();
    assert!(
        !by_domain_result.is_error.unwrap_or(false),
        "{by_domain_result:?}"
    );

    let no_scope_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("validate_project")
                .with_arguments(args_obj(&[])),
        )
        .await
        .unwrap_err();
    match no_scope_result {
        rmcp::ServiceError::McpError(e) => {
            assert_eq!(e.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

// delete_by_query: dry_run mode reports without deleting, then a real
// (default fail_if_referenced) mode deletes the unreferenced entity.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_delete_by_query_dry_run_then_real() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;
    let mut grpc = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();

    let ns = "mcp-gap-delete-by-query";
    grpc.put_entity(put_req(
        event_entity(ns, "unreferenced", 1, "Unreferenced"),
        true,
    ))
    .await
    .unwrap();

    let dry_run_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("delete_by_query").with_arguments(args_obj(&[
                ("namespace", serde_json::json!(ns)),
                ("kind", serde_json::json!("event")),
                ("mode", serde_json::json!("dry_run")),
                ("max_deletes", serde_json::json!(10)),
            ])),
        )
        .await
        .unwrap();
    assert!(
        !dry_run_result.is_error.unwrap_or(false),
        "{dry_run_result:?}"
    );
    let dry_run_report = call_tool_result_text(&dry_run_result);
    assert_eq!(
        dry_run_report["deletedCount"],
        serde_json::json!(1),
        "{dry_run_report:?}"
    );
    assert_eq!(
        dry_run_report["dryRun"],
        serde_json::json!(true),
        "{dry_run_report:?}"
    );

    // dry_run did not delete: get_entity still finds it.
    let mut grpc_check = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();
    grpc_check
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: "unreferenced".into(),
                version: 1,
            }),
        })
        .await
        .unwrap();

    let real_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("delete_by_query").with_arguments(args_obj(&[
                ("namespace", serde_json::json!(ns)),
                ("kind", serde_json::json!("event")),
                ("max_deletes", serde_json::json!(10)),
            ])),
        )
        .await
        .unwrap();
    assert!(!real_result.is_error.unwrap_or(false), "{real_result:?}");
    let real_report = call_tool_result_text(&real_result);
    assert_eq!(
        real_report["deletedCount"],
        serde_json::json!(1),
        "{real_report:?}"
    );

    let now_missing = grpc
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: "unreferenced".into(),
                version: 1,
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(now_missing.code(), tonic::Code::NotFound, "{now_missing:?}");

    mcp_service.cancel().await.unwrap();
}

// get_snapshot_id: the agent-facing form of G10. The value of the tool is
// the before/after comparison, so that is what the test exercises rather
// than merely decoding an envelope.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_get_snapshot_id_detects_change() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;
    let mut grpc =
        trogon_atlas_proto::event_model_service_client::EventModelServiceClient::connect(
            grpc_endpoint.clone(),
        )
        .await
        .unwrap();
    let ns = "mcp-snapid";

    let snapshot = |args: Vec<(&'static str, serde_json::Value)>| {
        let client = &client;
        async move {
            let result = client
                .call_tool(
                    rmcp::model::CallToolRequestParams::new("get_snapshot_id")
                        .with_arguments(args_obj(&args)),
                )
                .await
                .unwrap();
            assert!(!result.is_error.unwrap_or(false), "{result:?}");
            support::structured_as::<pb::GetSnapshotIdResponse>(&result, "GetSnapshotIdResponse")
        }
    };

    let scope = vec![("namespaces", serde_json::json!([ns]))];
    let empty = snapshot(scope.clone()).await;
    assert_eq!(empty.entity_count, 0);
    assert_eq!(empty.snapshot_id.len(), 64);
    assert!(empty.entries.is_empty(), "entries are off by default");

    grpc.put_entity(put_req(event_entity(ns, "a", 1, "A"), true))
        .await
        .unwrap();
    let after = snapshot(scope.clone()).await;
    assert_eq!(after.entity_count, 1);
    assert_ne!(after.snapshot_id, empty.snapshot_id);

    let detailed = snapshot(vec![
        ("namespaces", serde_json::json!([ns])),
        ("include_entries", serde_json::json!(true)),
    ])
    .await;
    assert_eq!(detailed.snapshot_id, after.snapshot_id);
    assert_eq!(detailed.entries.len(), 1);
    assert_eq!(detailed.entries[0].content_hash.len(), 64);

    mcp_service.cancel().await.unwrap();
}
