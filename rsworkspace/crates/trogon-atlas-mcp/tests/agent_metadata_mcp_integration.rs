#![allow(clippy::unwrap_used)]
#[allow(clippy::expect_used)]
mod support;
use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
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
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

use serde_json::json;
fn fixture() -> serde_json::Value {
    let target = json!({"kind":"ENTITY_KIND_COMPONENT","id":{"namespace":"catalog","slug":"target","version":"18446744073709551615"}});
    let raw = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Component","metadata":{"namespace":"catalog","name":"agent","version":"9007199254740993"},"spec":{"title":"Agent","metadata":[
        {"@type":"type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation","model":{"version":{"value":"opaque preview"}},"tools":[{"name":{"value":"lookup"},"type":{"value":"manual"},"description":{"value":"Inert description"},"icon":{"value":"unknown:token"},"url":{"value":"https://example.test/tool"}},{"name":{"value":"lookup"},"type":{"value":"other"}}]},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation","status":"draft","draft":{"message":{"value":"Review required"}}},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.BadgesAnnotation","badges":[{"content":{"value":"Preview"},"backgroundColor":{"value":"#aAbBcC"},"textColor":{"value":"#FFFFFF"}}]},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation","groups":[{"groupId":{"value":"10000000-0000-4000-8000-000000000001"},"displayLimit":{"value":1},"sidebar":false,"items":[{"connectionId":{"value":"10000000-0000-4000-8000-000000000002"},"target":target},{"connectionId":{"value":"10000000-0000-4000-8000-000000000003"},"target":target}]}]}
    ]}});
    raw
}

#[tokio::test(flavor = "multi_thread")]
async fn native_metadata_mcp_apply_get_and_reapply() {
    let (client, service, _) = start_mcp_client().await;
    let target = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Component","metadata":{"namespace":"catalog","name":"target","version":"18446744073709551615"},"spec":{"title":"Target"}});
    for manifest in [target, fixture(), fixture()] {
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("apply_manifests")
                    .with_arguments(args_obj(&[("manifests_yaml", json!(manifest.to_string()))])),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{result:?}");
    }
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity_json").with_arguments(args_obj(&[
                ("kind", json!("component")),
                ("namespace", json!("catalog")),
                ("slug", json!("agent")),
                ("version", json!(9_007_199_254_740_993_u64)),
            ])),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let body = call_tool_result_text(&result);
    assert_eq!(
        body["json"]["component"]["metadata"][3]["groups"][0]["sidebar"],
        false
    );
    assert_eq!(
        body["json"]["component"]["metadata"][3]["groups"][0]["items"][1]["target"]["id"]
            ["version"],
        "18446744073709551615"
    );
    let mut invalid = fixture();
    invalid["spec"]["metadata"][0]["model"] = json!({});
    let rejected = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("apply_manifests")
                .with_arguments(args_obj(&[("manifests_yaml", json!(invalid.to_string()))])),
        )
        .await
        .unwrap_err();
    assert!(
        rejected.to_string().contains("invalid metadata"),
        "{rejected:?}"
    );
    let unchanged = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity_json").with_arguments(args_obj(&[
                ("kind", json!("component")),
                ("namespace", json!("catalog")),
                ("slug", json!("agent")),
                ("version", json!(9_007_199_254_740_993_u64)),
            ])),
        )
        .await
        .unwrap();
    assert_eq!(call_tool_result_text(&unchanged), body);
    let diff = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("diff_manifests").with_arguments(args_obj(&[
                ("manifests_yaml", json!(fixture().to_string())),
            ])),
        )
        .await
        .unwrap();
    assert_eq!(call_tool_result_text(&diff)["changed"], false);
    use base64::Engine as _;
    use prost::Message as _;
    let mut missing_item_identity = Vec::new();
    for (tool, slug, version) in [
        (
            "get_outgoing_references",
            "agent",
            9_007_199_254_740_993_u64,
        ),
        ("get_incoming_references", "target", u64::MAX),
    ] {
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new(tool).with_arguments(args_obj(&[
                    ("kind", json!("component")),
                    ("namespace", json!("catalog")),
                    ("slug", json!(slug)),
                    ("version", json!(version)),
                ])),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{result:?}");
        let payload = call_tool_result_text(&result);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload["binpb_base64"].as_str().unwrap())
            .unwrap();
        let refs = trogon_atlas_proto::GetReferencesResponse::decode(bytes.as_slice()).unwrap();
        assert_eq!(refs.references.len(), 2);
        assert_ne!(refs.references[0].from_field, refs.references[1].from_field);
        for reference in refs.references {
            assert!(
                !reference.from_uid.is_empty(),
                "missing authoritative owner UID: {reference:?}"
            );
            if reference.connection_id.is_none() {
                missing_item_identity.push(reference);
            }
        }
    }
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_impact").with_arguments(args_obj(&[
                ("kind", json!("component")),
                ("namespace", json!("catalog")),
                ("slug", json!("target")),
                ("version", json!(u64::MAX)),
                ("max_depth", json!(1)),
            ])),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let payload = call_tool_result_text(&result);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload["binpb_base64"].as_str().unwrap())
        .unwrap();
    let impact = trogon_atlas_proto::GetImpactResponse::decode(bytes.as_slice()).unwrap();
    assert!(impact.nodes.iter().any(|node| node.depth == 1
        && node.entity.as_ref().unwrap().id.as_ref().unwrap().slug == "agent"));
    assert_eq!(
        missing_item_identity,
        [],
        "backend must return item provenance"
    );
    client.cancel().await.unwrap();
    service.cancel().await.unwrap();
}

fn changed_fixture(namespace: &str) -> serde_json::Value {
    let mut manifest = fixture();
    manifest["metadata"]["namespace"] = json!(namespace);
    let target = json!({"kind":"ENTITY_KIND_COMPONENT","id":{"namespace":namespace,"slug":"target","version":"18446744073709551615"}});
    let metadata = manifest["spec"]["metadata"].as_array_mut().unwrap();
    metadata[3]["groups"][0]["items"] = json!([
        {"connectionId":{"value":"10000000-0000-4000-8000-000000000002"},"target":target},
        {"connectionId":{"value":"10000000-0000-4000-8000-000000000003"},"target":target}
    ]);
    manifest
}

async fn tool(
    client: &McpClient,
    name: &'static str,
    pairs: &[(&str, serde_json::Value)],
) -> Result<rmcp::model::CallToolResult, rmcp::service::ServiceError> {
    client
        .call_tool(rmcp::model::CallToolRequestParams::new(name).with_arguments(args_obj(pairs)))
        .await
}

async fn stored_metadata(client: &McpClient, namespace: &str) -> serde_json::Value {
    let result = tool(
        client,
        "get_entity_json",
        &[
            ("kind", json!("component")),
            ("namespace", json!(namespace)),
            ("slug", json!("agent")),
            ("version", json!(9_007_199_254_740_993_u64)),
        ],
    )
    .await
    .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    call_tool_result_text(&result)["json"]["component"]["metadata"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn native_metadata_mcp_changed_update_is_diffed_applied_and_semantically_equal() {
    let (client, service, _) = start_mcp_client().await;
    let namespace = "catalog-changed-update";
    let original = changed_fixture(namespace);
    let target = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Component","metadata":{"namespace":namespace,"name":"target","version":"18446744073709551615"},"spec":{"title":"Target"}});
    for manifest in [&target, &original] {
        let result = tool(
            &client,
            "apply_manifests",
            &[("manifests_yaml", json!(manifest.to_string()))],
        )
        .await
        .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{result:?}");
    }
    let before = stored_metadata(&client, namespace).await;
    assert_eq!(before, original["spec"]["metadata"]);

    let mut changed = original.clone();
    let metadata = changed["spec"]["metadata"].as_array_mut().unwrap();
    metadata[0]["model"] = json!({"provider":{"value":"example"},"name":{"value":"model-b"}});
    let tools = metadata[0]["tools"].as_array_mut().unwrap();
    tools.reverse();
    tools.push(json!({"name":{"value":"extra"},"type":{"value":"manual"}}));
    metadata[1] = json!({"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation","status":"deprecated","deprecatedOn":{"value":"9999-12-31"}});
    metadata[2]["badges"] = json!([
        {"content":{"value":"Preview"},"backgroundColor":{"value":"#112233"},"textColor":{"value":"#FFFFFF"},"icon":{"value":"star"},"url":{"value":"https://example.test/badge"}},
        {"content":{"value":"Preview"},"backgroundColor":{"value":"#aAbBcC"},"textColor":{"value":"#FFFFFF"}}
    ]);
    metadata[3]["groups"][0]["items"]
        .as_array_mut()
        .unwrap()
        .reverse();
    metadata[3]["groups"][0]["displayLimit"] = json!({"value":2});
    metadata[3]["groups"][0]["sidebar"] = json!(true);

    let diff = tool(
        &client,
        "diff_manifests",
        &[("manifests_yaml", json!(changed.to_string()))],
    )
    .await
    .unwrap();
    assert_eq!(call_tool_result_text(&diff)["changed"], true);
    assert_eq!(stored_metadata(&client, namespace).await, before);

    let applied = tool(
        &client,
        "apply_manifests",
        &[("manifests_yaml", json!(changed.to_string()))],
    )
    .await
    .unwrap();
    assert!(!applied.is_error.unwrap_or(false), "{applied:?}");
    let after = stored_metadata(&client, namespace).await;
    assert_eq!(after, changed["spec"]["metadata"]);
    assert_ne!(after, before);
    let types: Vec<_> = after
        .as_array()
        .unwrap()
        .iter()
        .map(|annotation| annotation["@type"].clone())
        .collect();
    assert_eq!(
        types,
        original["spec"]["metadata"]
            .as_array()
            .unwrap()
            .iter()
            .map(|annotation| annotation["@type"].clone())
            .collect::<Vec<_>>()
    );

    for (manifest, expected) in [(&changed, false), (&original, true)] {
        let diff = tool(
            &client,
            "diff_manifests",
            &[("manifests_yaml", json!(manifest.to_string()))],
        )
        .await
        .unwrap();
        assert_eq!(call_tool_result_text(&diff)["changed"], expected);
    }
    client.cancel().await.unwrap();
    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn schema_valid_but_semantically_malformed_annotations_are_rejected_by_the_backend() {
    let (client, service, _) = start_mcp_client().await;
    let namespace = "catalog-schema-valid";
    let validator =
        jsonschema::validator_for(&trogon_atlas_client::manifest_schema::generate().unwrap())
            .unwrap();
    let valid = changed_fixture(namespace);
    let target = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Component","metadata":{"namespace":namespace,"name":"target","version":"18446744073709551615"},"spec":{"title":"Target"}});
    for manifest in [&target, &valid] {
        let result = tool(
            &client,
            "apply_manifests",
            &[("manifests_yaml", json!(manifest.to_string()))],
        )
        .await
        .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{result:?}");
    }
    let before = stored_metadata(&client, namespace).await;
    let tool_a = json!({"name":{"value":"lookup"},"type":{"value":"manual"}});
    type MetadataEdit = (&'static str, usize, Box<dyn Fn(&mut serde_json::Value)>);
    let edits: Vec<MetadataEdit> = vec![
        (
            "empty present model",
            0,
            Box::new(|a| a["model"] = json!({})),
        ),
        (
            "duplicate tool pair",
            0,
            Box::new(move |a| a["tools"] = json!([tool_a.clone(), tool_a.clone()])),
        ),
        (
            "plain http tool URL",
            0,
            Box::new(|a| {
                a["tools"][0]["url"] = json!({"value":"http://example.test/tool"});
            }),
        ),
        (
            "blank draft message",
            1,
            Box::new(|a| a["draft"]["message"] = json!({"value":"  "})),
        ),
        (
            "date without deprecated status",
            1,
            Box::new(|a| a["deprecatedOn"] = json!({"value":"2030-01-01"})),
        ),
        (
            "impossible calendar date",
            1,
            Box::new(|a| {
                a["status"] = json!("deprecated");
                a["draft"] = json!(null);
                a["deprecatedOn"] = json!({"value":"2026-02-30"});
            }),
        ),
        (
            "named color",
            2,
            Box::new(|a| a["badges"][0]["backgroundColor"] = json!({"value":"red"})),
        ),
        (
            "identical badges",
            2,
            Box::new(|a| {
                let first = a["badges"][0].clone();
                a["badges"] = json!([first.clone(), first]);
            }),
        ),
        (
            "zero display limit",
            3,
            Box::new(|a| a["groups"][0]["displayLimit"] = json!({"value":0})),
        ),
        (
            "duplicate item ID",
            3,
            Box::new(|a| {
                a["groups"][0]["items"][1]["connectionId"] =
                    a["groups"][0]["items"][0]["connectionId"].clone();
            }),
        ),
        (
            "latest-version selector",
            3,
            Box::new(|a| {
                a["groups"][0]["items"][0]["target"]["id"]["version"] = json!("0");
            }),
        ),
    ];
    for (name, index, edit) in edits {
        let mut manifest = valid.clone();
        let annotation = &mut manifest["spec"]["metadata"][index];
        edit(annotation);
        if annotation["draft"].is_null() {
            annotation.as_object_mut().unwrap().remove("draft");
        }
        assert!(
            validator.is_valid(&manifest),
            "{name}: the schema must accept it so only the backend can refuse it"
        );
        let rejected = tool(
            &client,
            "apply_manifests",
            &[("manifests_yaml", json!(manifest.to_string()))],
        )
        .await;
        let refused = match rejected {
            Err(error) => error.to_string().contains("invalid metadata"),
            Ok(result) => result.is_error.unwrap_or(false),
        };
        assert!(refused, "{name}: the backend accepted {manifest}");
        assert_eq!(stored_metadata(&client, namespace).await, before, "{name}");
    }
    client.cancel().await.unwrap();
    service.cancel().await.unwrap();
}
