#![allow(clippy::unwrap_used, clippy::expect_used)]

// Full MCP stdio-transport round trips for the tools left uncovered by
// mcp_integration.rs: export_namespace, apply_manifests
// (dry_run), diff_manifests, validate_event_model, list_entities,
// get_entity/get_entity_json, put_entity_json, delete_entity,
// batch_mutate_json, list_changes. Same duplex-pipe harness as
// mcp_integration.rs's "Full MCP transport dispatch" section (a fresh
// server/client pair against a fresh in-process gRPC backend, driven by
// rmcp::serve_client over tokio::io::duplex).
//
// search_entities is intentionally skipped: it depends on the search index
// build, which needs more setup than the other tools justify here.

use std::sync::Arc;

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

fn event_json(ns: &str, slug: &str, version: u64, title: &str) -> String {
    serde_json::json!({
        "event": {
            "id": {"namespace": ns, "slug": slug, "version": version},
            "title": title,
        }
    })
    .to_string()
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

// put_entity_json / get_entity_json / get_entity (binpb_base64) / list_entities
// / delete_entity / batch_mutate_json / list_changes, chained through one
// lifecycle so the whole read/write surface gets exercised in one server
// instance.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_read_write_tool_lifecycle() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;

    // put_entity_json: create.
    let put_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity_json").with_arguments(args_obj(&[
                (
                    "entity_json",
                    serde_json::json!(event_json("mcp-tools", "created", 1, "Created")),
                ),
                ("create_only", serde_json::json!(true)),
            ])),
        )
        .await
        .unwrap();
    assert!(!put_result.is_error.unwrap_or(false), "{put_result:?}");
    let put_body = call_tool_result_text(&put_result);
    assert!(put_body["binpb_base64"].is_string(), "{put_body:?}");

    // get_entity_json: read back as proto3 JSON, with an etag.
    let get_json_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity_json").with_arguments(args_obj(&[
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!("mcp-tools")),
                ("slug", serde_json::json!("created")),
                ("version", serde_json::json!(1)),
            ])),
        )
        .await
        .unwrap();
    assert!(
        !get_json_result.is_error.unwrap_or(false),
        "{get_json_result:?}"
    );
    let get_json_body = call_tool_result_text(&get_json_result);
    assert!(
        !get_json_body["etag"].as_str().unwrap().is_empty(),
        "{get_json_body:?}"
    );
    assert_eq!(
        get_json_body["json"]["event"]["title"],
        serde_json::json!("Created")
    );

    // get_entity: raw binpb_base64 envelope, decodable as a GetEntityResponse.
    let get_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity").with_arguments(args_obj(&[
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!("mcp-tools")),
                ("slug", serde_json::json!("created")),
                ("version", serde_json::json!(1)),
            ])),
        )
        .await
        .unwrap();
    assert!(!get_result.is_error.unwrap_or(false), "{get_result:?}");
    let get_body = call_tool_result_text(&get_result);
    let raw = base64_decode(get_body["binpb_base64"].as_str().unwrap());
    let decoded = pb::GetEntityResponse::decode(raw.as_slice()).unwrap();
    match decoded.entity.unwrap().kind {
        Some(pb::entity::Kind::Event(ev)) => assert_eq!(ev.title, "Created"),
        other => panic!("unexpected: {other:?}"),
    }

    // list_entities: filtered by kind/namespace, finds the entity just made.
    let list_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("list_entities").with_arguments(args_obj(&[
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!("mcp-tools")),
                ("page_size", serde_json::json!(100)),
            ])),
        )
        .await
        .unwrap();
    assert!(!list_result.is_error.unwrap_or(false), "{list_result:?}");
    let list_body = call_tool_result_text(&list_result);
    let raw = base64_decode(list_body["binpb_base64"].as_str().unwrap());
    let decoded = pb::ListEntitiesResponse::decode(raw.as_slice()).unwrap();
    assert_eq!(decoded.entities.len(), 1, "{decoded:?}");

    // batch_mutate_json: put a second entity and delete the first, atomically.
    let batch_request_json = serde_json::json!({
        "ops": [
            {"put": {"entity": {"event": {"id": {"namespace": "mcp-tools", "slug": "second", "version": 1}, "title": "Second"}}, "createOnly": true}},
            {"delete": {"kind": "ENTITY_KIND_EVENT", "id": {"namespace": "mcp-tools", "slug": "created", "version": 1}}},
        ]
    })
    .to_string();
    let batch_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("batch_mutate_json").with_arguments(args_obj(
                &[("request_json", serde_json::json!(batch_request_json))],
            )),
        )
        .await
        .unwrap();
    assert!(!batch_result.is_error.unwrap_or(false), "{batch_result:?}");
    let batch_body = call_tool_result_text(&batch_result);
    let raw = base64_decode(batch_body["binpb_base64"].as_str().unwrap());
    let decoded = pb::BatchMutateResponse::decode(raw.as_slice()).unwrap();
    assert_eq!(decoded.results.len(), 2, "{decoded:?}");

    // delete_entity: the second entity is now deletable directly.
    let delete_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("delete_entity").with_arguments(args_obj(&[
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!("mcp-tools")),
                ("slug", serde_json::json!("second")),
                ("version", serde_json::json!(1)),
            ])),
        )
        .await
        .unwrap();
    assert!(
        !delete_result.is_error.unwrap_or(false),
        "{delete_result:?}"
    );

    // The batch delete already removed "created": get_entity_json for it
    // now surfaces the classified NOT_FOUND error path.
    let missing = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity_json").with_arguments(args_obj(&[
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!("mcp-tools")),
                ("slug", serde_json::json!("created")),
                ("version", serde_json::json!(1)),
            ])),
        )
        .await
        .unwrap_err();
    match missing {
        rmcp::ServiceError::McpError(e) => {
            assert_eq!(e.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    // list_changes: empty since_token returns an opaque cursor positioned
    // at "now" (no events, since every mutation above already happened
    // before this call). Passing that cursor back as since_token then picks
    // up a fresh write made after it, proving the paged change feed works
    // end-to-end through the tool surface.
    let first_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("list_changes")
                .with_arguments(args_obj(&[("page_size", serde_json::json!(50))])),
        )
        .await
        .unwrap();
    assert!(!first_result.is_error.unwrap_or(false), "{first_result:?}");
    let first_body = call_tool_result_text(&first_result);
    let raw = base64_decode(first_body["binpb_base64"].as_str().unwrap());
    let first_decoded = pb::ListChangesResponse::decode(raw.as_slice()).unwrap();
    assert!(!first_decoded.next_token.is_empty(), "{first_decoded:?}");

    let mut grpc = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();
    grpc.put_entity(put_req(
        event_entity("mcp-tools", "after-cursor", 1, "After cursor"),
        true,
    ))
    .await
    .unwrap();

    let second_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("list_changes").with_arguments(args_obj(&[
                ("since_token", serde_json::json!(first_decoded.next_token)),
                ("page_size", serde_json::json!(50)),
            ])),
        )
        .await
        .unwrap();
    assert!(
        !second_result.is_error.unwrap_or(false),
        "{second_result:?}"
    );
    let second_body = call_tool_result_text(&second_result);
    let raw = base64_decode(second_body["binpb_base64"].as_str().unwrap());
    let second_decoded = pb::ListChangesResponse::decode(raw.as_slice()).unwrap();
    assert!(!second_decoded.events.is_empty(), "{second_decoded:?}");

    mcp_service.cancel().await.unwrap();
    let _ = grpc_endpoint;
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

// export_namespace: apply a small model via apply_manifests (dry_run, so the
// plan/diff paths execute too), then a real apply via raw put_entity, then
// export_namespace confirms the grouping/skip report.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_apply_diff_export_lifecycle() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;

    let manifest_yaml = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: mcp-export-ns, name: order.placed }
spec: { title: Order placed }
";

    // apply_manifests dry_run=true must validate without persisting.
    let dry_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("apply_manifests").with_arguments(args_obj(&[
                ("manifests_yaml", serde_json::json!(manifest_yaml)),
                ("dry_run", serde_json::json!(true)),
            ])),
        )
        .await
        .unwrap();
    assert!(!dry_result.is_error.unwrap_or(false), "{dry_result:?}");
    let dry_body = call_tool_result_text(&dry_result);
    let lines = dry_body["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1, "{dry_body:?}");
    assert!(
        lines[0].as_str().unwrap().contains("dry run"),
        "{:?}",
        lines[0]
    );

    // diff_manifests: dry run above did not persist, so live state still
    // differs from the manifest.
    let diff_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("diff_manifests").with_arguments(args_obj(&[
                ("manifests_yaml", serde_json::json!(manifest_yaml)),
            ])),
        )
        .await
        .unwrap();
    assert!(!diff_result.is_error.unwrap_or(false), "{diff_result:?}");
    let diff_body = call_tool_result_text(&diff_result);
    assert_eq!(
        diff_body["changed"],
        serde_json::json!(true),
        "{diff_body:?}"
    );

    // A real (non-dry-run) apply persists it.
    let apply_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("apply_manifests").with_arguments(args_obj(&[
                ("manifests_yaml", serde_json::json!(manifest_yaml)),
            ])),
        )
        .await
        .unwrap();
    assert!(!apply_result.is_error.unwrap_or(false), "{apply_result:?}");
    let apply_body = call_tool_result_text(&apply_result);
    assert!(
        apply_body["lines"][0]
            .as_str()
            .unwrap()
            .ends_with(" created"),
        "{apply_body:?}"
    );

    // Now diff_manifests against the same manifest reports no drift.
    let diff_after = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("diff_manifests").with_arguments(args_obj(&[
                ("manifests_yaml", serde_json::json!(manifest_yaml)),
            ])),
        )
        .await
        .unwrap();
    let diff_after_body = call_tool_result_text(&diff_after);
    assert_eq!(
        diff_after_body["changed"],
        serde_json::json!(false),
        "{diff_after_body:?}"
    );

    // export_namespace: renders the applied manifest back out.
    let export_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("export_namespace").with_arguments(args_obj(
                &[("namespace", serde_json::json!("mcp-export-ns"))],
            )),
        )
        .await
        .unwrap();
    assert!(
        !export_result.is_error.unwrap_or(false),
        "{export_result:?}"
    );
    let export_body = call_tool_result_text(&export_result);
    let files = export_body["files"].as_object().unwrap();
    assert!(files.contains_key("event.yaml"), "{:?}", files.keys());
    assert!(
        files["event.yaml"]
            .as_str()
            .unwrap()
            .contains("order.placed"),
        "{:?}",
        files["event.yaml"]
    );
    assert!(
        export_body["skipped"].as_array().unwrap().is_empty(),
        "{export_body:?}"
    );

    // export_namespace on a namespace with no entities surfaces as an error
    // through the tool boundary (export_namespace bails on empty results).
    // The bail is `InvalidInput`, a caller-input problem, so it classifies
    // as `validation` and keeps its message instead of being replaced with
    // a generic one.
    let export_missing = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("export_namespace").with_arguments(args_obj(
                &[("namespace", serde_json::json!("mcp-export-does-not-exist"))],
            )),
        )
        .await
        .unwrap_err();
    match export_missing {
        rmcp::ServiceError::McpError(e) => {
            let data = e.data.as_ref().expect("classified error carries data");
            assert_eq!(data["category"], "validation", "{e:?}");
            assert_eq!(data["next_action"], "fix_input", "{e:?}");
            assert!(e.message.contains("no entities"), "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
    let _ = grpc_endpoint;
}

// validate_event_model: happy path (no issues on an empty-membership model)
// via the tool surface, requesting JSON rendering.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_validate_event_model_returns_json_report() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;

    let mut grpc = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();
    grpc.put_entity(put_req(
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
                id: Some(pb::Id {
                    namespace: "mcp-validate".into(),
                    slug: "model".into(),
                    version: 1,
                }),
                title: "Model".into(),
                members: Vec::new(),
                ..Default::default()
            })),
        },
        true,
    ))
    .await
    .unwrap();

    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("validate_event_model").with_arguments(
                args_obj(&[
                    ("namespace", serde_json::json!("mcp-validate")),
                    ("slug", serde_json::json!("model")),
                    ("version", serde_json::json!(1)),
                    ("json", serde_json::json!(true)),
                ]),
            ),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let body = call_tool_result_text(&result);
    assert_eq!(body["format"], serde_json::json!("json"), "{body:?}");
    let report: serde_json::Value = serde_json::from_str(body["json"].as_str().unwrap()).unwrap();
    // proto3 JSON omits empty repeated fields: absent means no issues.
    assert!(report
        .get("reports")
        .is_none_or(|r| r.as_array().is_none_or(Vec::is_empty)));

    mcp_service.cancel().await.unwrap();
}

// batch_mutate_json: a batch that the server reports as STATUS_FAILED (here,
// deleting an entity that does not exist) must surface through the tool
// boundary as an MCP error carrying classification data, the same contract
// batch_mutate already gives -- never a success envelope with a failed
// status buried inside its binpb payload.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_batch_mutate_json_status_failed_reports_as_error() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let batch_request_json = serde_json::json!({
        "ops": [
            {"delete": {"kind": "ENTITY_KIND_EVENT", "id": {"namespace": "mcp-batch-failed", "slug": "missing", "version": 1}}},
        ]
    })
    .to_string();
    let batch_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("batch_mutate_json").with_arguments(args_obj(
                &[("request_json", serde_json::json!(batch_request_json))],
            )),
        )
        .await
        .unwrap_err();
    match batch_result {
        rmcp::ServiceError::McpError(e) => {
            let data = e.data.as_ref().expect("classified error carries data");
            assert_eq!(data["category"], "not_found", "{e:?}");
            assert_eq!(data["code"], "ENTITY_NOT_FOUND", "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}
