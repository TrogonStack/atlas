#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServiceRunning =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

async fn mcp_client(endpoint: &str) -> (McpClient, McpServiceRunning) {
    let mcp_server = support::mcp_server(endpoint, None);
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    use rmcp::ServiceExt;
    let (service, client) = tokio::join!(
        mcp_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client.unwrap(), service.unwrap())
}

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn args(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

async fn tool(
    client: &McpClient,
    name: &'static str,
    arguments: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new(name).with_arguments(arguments))
        .await
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
    assert!(!result.is_error.unwrap_or(false), "{name}: {result:?}");
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

async fn apply(client: &McpClient, manifests: &serde_json::Value) {
    let body = tool(
        client,
        "apply_manifests",
        args(&[("manifests_yaml", serde_json::json!(manifests.to_string()))]),
    )
    .await;
    assert_eq!(body["issues"], serde_json::json!([]), "{body}");
}

fn id(namespace: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version,
    }
}

async fn stored(endpoint: &str, kind: pb::EntityKind, namespace: &str, slug: &str) -> pb::Entity {
    let mut client =
        trogon_atlas_proto::event_model_service_client::EventModelServiceClient::connect(
            endpoint.to_string(),
        )
        .await
        .unwrap();
    client
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(id(namespace, slug, if slug == "receipt" { 2 } else { 1 })),
        })
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn etag(endpoint: &str, kind: pb::EntityKind, namespace: &str, slug: &str) -> String {
    let mut client =
        trogon_atlas_proto::event_model_service_client::EventModelServiceClient::connect(
            endpoint.to_string(),
        )
        .await
        .unwrap();
    client
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(id(namespace, slug, 1)),
        })
        .await
        .unwrap()
        .into_inner()
        .etag
}

fn manifest(namespace: &str, kind: &str, name: &str, spec: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": "eventmodel.atlas.trogonstack.com/v1alpha1",
        "kind": kind,
        "metadata": {"namespace": namespace, "name": name, "version": if name == "receipt" { serde_json::json!(2) } else { serde_json::json!(1) }},
        "spec": spec
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_authoring_inspect_diff_export_and_reapply_keep_native_declarations() {
    let endpoint = start_server().await;
    let (client, service) = mcp_client(&endpoint).await;
    let ns = unique("assoc-mcp");
    let environment = "a".repeat(64);
    let anchors = serde_json::json!([
        manifest(
            &ns,
            "Event",
            "placed",
            serde_json::json!({"title": "Placed"})
        ),
        manifest(
            &ns,
            "Command",
            "place",
            serde_json::json!({"title": "Place"})
        ),
        manifest(
            &ns,
            "Schema",
            "shape",
            serde_json::json!({"title": "Shape"})
        ),
        manifest(
            &ns,
            "ReadModel",
            "receipt",
            serde_json::json!({"title": "Receipt"})
        ),
        manifest(
            &ns,
            "Processor",
            "worker",
            serde_json::json!({"title": "Worker"})
        ),
        manifest(
            &ns,
            "Component",
            "checkout",
            serde_json::json!({"title": "Checkout"})
        ),
        manifest(
            &ns,
            "ExternalSystem",
            "partner",
            serde_json::json!({"title": "Partner"})
        ),
        manifest(
            &ns,
            "DataStore",
            "cache",
            serde_json::json!({
                "title": "Cache",
                "storeClass": {"known": "KNOWN_CACHE"},
                "authoritative": false,
                "attachments": [{"readModel": "receipt@2"}]
            })
        ),
        manifest(
            &ns,
            "AutomationSlice",
            "observe",
            serde_json::json!({
                "title": "Observe",
                "sourceReadModels": ["receipt@2"],
                "processor": "worker"
            })
        )
    ]);
    for document in anchors.as_array().unwrap() {
        apply(&client, document).await;
    }
    let owner = stored(&endpoint, pb::EntityKind::AutomationSlice, &ns, "observe").await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = &owner.kind else {
        panic!("automation");
    };
    let observation = serde_json::json!({
        "owner": {"kind": "ENTITY_KIND_AUTOMATION_SLICE", "id": {"namespace": ns, "slug": "observe", "version": "1"}},
        "ownerUid": owner.system.as_ref().unwrap().uid,
        "id": {"value": slice.source_read_models[0].connection_id.as_ref().unwrap().value}
    });
    let channel = |name: &str| {
        manifest(
            &ns,
            "Channel",
            name,
            serde_json::json!({
                "title": name,
                "protocols": [{"value": "nats"}],
                "address": {
                    "template": "orders.{topic}.{note}",
                    "parameters": [{"name": {"value": "topic"}}, {"name": {"value": "note"}}]
                },
                "bindings": [{
                    "observation": observation,
                    "parameterValues": {"topic": "orders", "note": "kept"}
                }]
            }),
        )
    };
    apply(&client, &channel("orders")).await;
    apply(&client, &channel("backup")).await;

    let content = "{\"type\":\"object\"}\r\n";
    let digest = trogon_atlas_core::specification::content_digest(content).sha256;
    apply(
        &client,
        &manifest(
            &ns,
            "Specification",
            "contract",
            serde_json::json!({
                "title": "Contract",
                "artifacts": [{
                    "memberId": {"value": "44444444-4444-4444-8444-444444444444"},
                    "name": {"value": "Source"},
                    "path": {"value": "contract.txt"},
                    "format": {"other": "opaque"},
                    "mediaType": {"value": "text/plain"},
                    "content": content,
                    "digest": {"sha256": digest}
                }],
                "schemas": [{
                    "memberId": {"value": "33333333-3333-4333-8333-333333333333"},
                    "name": {"value": "Shape"},
                    "definition": {"artifact": {
                        "artifactId": {"value": "44444444-4444-4444-8444-444444444444"},
                        "expectedDigest": {"sha256": digest}
                    }}
                }],
                "messages": [
                    {
                        "memberId": {"value": "11111111-1111-4111-8111-111111111111"},
                        "name": {"value": "Order"},
                        "schemas": [
                            {
                                "role": "ROLE_PAYLOAD",
                                "localSchema": {"value": "33333333-3333-4333-8333-333333333333"},
                                "environments": [{"value": environment}],
                                "isDefault": false
                            },
                            {
                                "role": "ROLE_HEADERS",
                                "localSchema": {"value": "33333333-3333-4333-8333-333333333333"},
                                "environments": [{"value": environment}]
                            }
                        ]
                    },
                    {
                        "memberId": {"value": "22222222-2222-4222-8222-222222222222"},
                        "name": {"value": "Query intent"}
                    }
                ],
                "messageChannels": [
                    {
                        "memberId": {"value": "11111111-1111-4111-8111-111111111111"},
                        "channel": {"id": {"namespace": ns, "slug": "orders", "version": "1"}},
                        "parameterValues": {"topic": "orders", "note": ""},
                        "doc": "carried"
                    },
                    {
                        "memberId": {"value": "11111111-1111-4111-8111-111111111111"},
                        "channel": {"id": {"namespace": ns, "slug": "orders", "version": "1"}},
                        "parameterValues": {"topic": "orders", "note": ""},
                        "deliveryMode": "CLAIM_DELIVERY_MODE_PUSH"
                    }
                ]
            }),
        ),
    )
    .await;
    let stored_spec = stored(&endpoint, pb::EntityKind::Specification, &ns, "contract").await;
    let spec_uid = stored_spec.system.as_ref().unwrap().uid.clone();
    let spec_etag = etag(&endpoint, pb::EntityKind::Specification, &ns, "contract").await;
    let Some(pb::entity::Kind::Specification(contract)) = &stored_spec.kind else {
        panic!("specification");
    };
    assert_eq!(contract.messages[0].schemas[0].is_default, Some(false));
    assert!(contract.messages[0].schemas[1].is_default.is_none());
    assert!(contract.message_channels[0].delivery_mode.is_none());
    assert_eq!(
        contract.message_channels[1].delivery_mode,
        Some(pb::ClaimDeliveryMode::Push as i32)
    );
    assert_eq!(
        contract.message_channels[0]
            .parameter_values
            .get("note")
            .map(String::as_str),
        Some("")
    );

    let reference = |slug: &str| serde_json::json!({"id": {"namespace": ns, "slug": slug, "version": if slug == "receipt" { "2" } else { "1" }}});
    apply(
        &client,
        &manifest(&ns, "DataProduct", "offering", serde_json::json!({
            "title": "Offering",
            "outputs": [{"name": {"value": "Receipt"}, "data": {"readModel": "receipt@2"}}],
            "resourceAssociations": [
                {
                    "direction": "DATA_PORT_DIRECTION_OUTPUT",
                    "event": reference("placed"),
                    "contract": {"schema": reference("shape")},
                    "doc": "descriptive"
                },
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "event": reference("placed")},
                {"direction": "DATA_PORT_DIRECTION_INPUT", "command": reference("place")},
                {
                    "direction": "DATA_PORT_DIRECTION_OUTPUT",
                    "message": {
                        "specification": reference("contract"),
                        "ownerUid": spec_uid,
                        "kind": "CONTRACT_MEMBER_KIND_MESSAGE",
                        "memberId": {"value": "22222222-2222-4222-8222-222222222222"}
                    }
                },
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "component": reference("checkout")},
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "externalSystem": reference("partner")},
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "channel": reference("orders")},
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "dataStore": reference("cache")}
            ]
        })),
    )
    .await;
    let before = stored(&endpoint, pb::EntityKind::DataProduct, &ns, "offering").await;
    let before_uid = before.system.as_ref().unwrap().uid.clone();
    let Some(pb::entity::Kind::DataProduct(product)) = &before.kind else {
        panic!("product");
    };
    assert_eq!(product.outputs.len(), 1);
    assert_eq!(product.resource_associations.len(), 8);
    let ids: std::collections::HashSet<_> = product
        .resource_associations
        .iter()
        .map(|association| association.connection_id.as_ref().unwrap().value.clone())
        .collect();
    assert_eq!(ids.len(), 8);
    assert!(product.resource_associations[0].contract.is_some());
    assert!(product.resource_associations[1].contract.is_none());

    let spec_json = tool(
        &client,
        "get_entity_json",
        args(&[
            ("kind", serde_json::json!("specification")),
            ("namespace", serde_json::json!(ns)),
            ("slug", serde_json::json!("contract")),
            ("version", serde_json::json!(1)),
        ]),
    )
    .await;
    let viewed = &spec_json["json"]["specification"];
    assert_eq!(viewed["messages"][0]["schemas"][0]["isDefault"], false);
    assert!(viewed["messages"][0]["schemas"][1]
        .get("isDefault")
        .is_none());
    assert!(viewed["messageChannels"][0].get("deliveryMode").is_none());
    assert_eq!(
        viewed["messageChannels"][1]["deliveryMode"],
        "CLAIM_DELIVERY_MODE_PUSH"
    );
    assert_eq!(viewed["messageChannels"][0]["parameterValues"]["note"], "");
    let product_json = tool(
        &client,
        "get_entity_json",
        args(&[
            ("kind", serde_json::json!("data_product")),
            ("namespace", serde_json::json!(ns)),
            ("slug", serde_json::json!("offering")),
            ("version", serde_json::json!(1)),
        ]),
    )
    .await;
    assert_eq!(
        product_json["json"]["dataProduct"]["resourceAssociations"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    let inspection = tool(
        &client,
        "inspect_data_product",
        args(&[
            ("kind", serde_json::json!("data_product")),
            ("namespace", serde_json::json!(ns)),
            ("slug", serde_json::json!("offering")),
            ("version", serde_json::json!(1)),
        ]),
    )
    .await;
    assert_eq!(inspection["ports"].as_array().unwrap().len(), 1);

    let exported = tool(
        &client,
        "export_namespace",
        args(&[("namespace", serde_json::json!(ns))]),
    )
    .await;
    assert_eq!(exported["skipped"], serde_json::json!([]));
    let yaml = exported["files"]
        .as_object()
        .unwrap()
        .values()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n---\n");
    let diff = tool(
        &client,
        "diff_manifests",
        args(&[("manifests_yaml", serde_json::json!(yaml))]),
    )
    .await;
    assert_eq!(diff["changed"], false, "{diff}");
    let reapplied = tool(
        &client,
        "apply_manifests",
        args(&[("manifests_yaml", serde_json::json!(yaml))]),
    )
    .await;
    assert_eq!(reapplied["issues"], serde_json::json!([]), "{reapplied}");
    let lines = reapplied["lines"].to_string();
    assert!(!lines.contains("created"), "{lines}");
    assert!(!lines.contains("updated"), "{lines}");
    let after = stored(&endpoint, pb::EntityKind::DataProduct, &ns, "offering").await;
    assert_eq!(after.system.as_ref().unwrap().uid, before_uid);
    assert_eq!(after.kind, before.kind);
    assert_eq!(
        etag(&endpoint, pb::EntityKind::Specification, &ns, "contract").await,
        spec_etag
    );
    service.cancel().await.unwrap();
}
