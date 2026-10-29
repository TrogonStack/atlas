#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const VERSION: u64 = 9_007_199_254_740_993;

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

fn cli(endpoint: &str, args: &[&str]) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_schema_authoring_preserves_presence_versions_and_reapply_identity() {
    use serde_json::json;
    use trogon_atlas_proto as pb;

    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema.yaml");
    let product = json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {"namespace": "cli-business", "name": "product"},
        "spec": {
            "title": "Product",
            "fields": [{"name": "sku", "type": {"string": {}}}]
        }
    });
    std::fs::write(&path, product.to_string()).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let order = json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {
            "namespace": "cli-business",
            "name": "order",
            "version": VERSION.to_string()
        },
        "spec": {
            "title": "Order",
            "aggregateRoot": false,
            "fields": [
                {"name": "customer", "repeated": false, "type": {"object": {"fields": [
                    {"name": "id", "repeated": false, "type": {"string": {}}}
                ]}}},
                {"name": "lines", "repeated": true, "type": {"object": {"fields": [
                    {"name": "sku", "repeated": false, "type": {"string": {}}}
                ]}}},
                {"name": "product_ref", "type": {"ref": {"ref": {"schema": "product"}}}}
            ],
            "identifiers": [
                {
                    "property": {"segments": [
                        {"name": "customer", "repeated": false},
                        {"name": "id", "repeated": false}
                    ]},
                    "role": "BUSINESS_IDENTIFIER_ROLE_PRIMARY"
                },
                {
                    "property": {"segments": [
                        {"name": "lines", "repeated": true},
                        {"name": "sku", "repeated": false}
                    ]},
                    "role": "BUSINESS_IDENTIFIER_ROLE_ALTERNATE"
                }
            ],
            "propertyRelationships": [{
                "source": {"segments": [{"name": "product_ref", "repeated": false}]},
                "targetEntity": "product",
                "targetProperty": {"segments": [{"name": "sku", "repeated": false}]},
                "label": {"value": "hasMany"}
            }]
        }
    });
    std::fs::write(&path, order.to_string()).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);

    let address = format!("schema/cli-business/order@{VERSION}");
    let got = cli(&endpoint, &["--format", "json", "get", &address]);
    let got: serde_json::Value = serde_json::from_str(&got).unwrap();
    let manifest = &got["data"]["manifest"];
    assert_eq!(manifest["spec"]["aggregateRoot"], false);
    assert_eq!(manifest["metadata"]["version"], serde_json::json!(VERSION));
    assert_eq!(
        manifest["spec"]["identifiers"][1]["property"]["segments"][0]["repeated"],
        true
    );
    let loaded = trogon_atlas_client::manifest::parse_str(&manifest.to_string(), "get.json")
        .unwrap()
        .remove(0)
        .entity;
    let pb::entity::Kind::Schema(schema) = loaded.kind.as_ref().unwrap() else {
        panic!("schema");
    };
    assert_eq!(schema.aggregate_root, Some(false));
    assert!(!schema.identifiers[0].property.as_ref().unwrap().segments[0].repeated);
    assert!(schema.identifiers[1].property.as_ref().unwrap().segments[0].repeated);
    assert_eq!(schema.id.as_ref().unwrap().version, VERSION);
    let primary_id = manifest["spec"]["identifiers"][0]["connectionId"]["value"]
        .as_str()
        .expect("primary connection id");
    let alternate_id = manifest["spec"]["identifiers"][1]["connectionId"]["value"]
        .as_str()
        .expect("alternate connection id");
    let relationship_id = manifest["spec"]["propertyRelationships"][0]["connectionId"]["value"]
        .as_str()
        .expect("relationship connection id");
    assert_ne!(primary_id, alternate_id);
    let described = cli(&endpoint, &["--format", "json", "describe", &address]);
    let described: serde_json::Value = serde_json::from_str(&described).unwrap();
    let rendered = described.to_string();
    assert!(rendered.contains(relationship_id), "{described}");
    assert!(
        rendered.contains("property_relationships") || rendered.contains("propertyRelationships"),
        "{described}"
    );

    let mut native =
        pb::event_model_service_client::EventModelServiceClient::connect(endpoint.clone())
            .await
            .unwrap();
    let before = native
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(pb::Id {
                namespace: "cli-business".into(),
                slug: "order".into(),
                version: VERSION,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    let uid = before
        .entity
        .as_ref()
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone();
    assert_ne!(uid, "");

    let export = dir.path().join("export");
    cli(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-business",
            "--output",
            export.to_str().unwrap(),
        ],
    );
    cli(&endpoint, &["diff", "-f", export.to_str().unwrap()]);
    cli(&endpoint, &["apply", "-f", export.to_str().unwrap()]);
    let after = native
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(pb::Id {
                namespace: "cli-business".into(),
                slug: "order".into(),
                version: VERSION,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after, before);

    let missing = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            &endpoint,
            "--branch",
            "bd-missing",
            "get",
            &address,
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("branch not found"), "{stderr}");
    assert!(!stderr.contains(&uid));
}

const WRITER: &str = "cli-accept-writer";
const READER: &str = "cli-accept-reader";

async fn authenticated_server() -> String {
    use serde_json::json;
    use trogon_atlas_server::{
        auth::{BearerAuth, TokenRegistry},
        mcp::data_plane,
    };

    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "writer": {"role": "writer", "tokens": [WRITER]},
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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let secured = data_plane(
        EventModelServiceServer::new(service)
            .max_decoding_message_size(36 * 1024 * 1024)
            .max_encoding_message_size(36 * 1024 * 1024),
        BearerAuth::new(registry),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(secured)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{address}")
}

fn authed(endpoint: &str, token: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            endpoint,
            "--auth-token",
            token,
            "--format",
            "json",
        ])
        .args(args)
        .output()
        .unwrap()
}

fn authed_ok(endpoint: &str, token: &str, args: &[&str]) -> serde_json::Value {
    let output = authed(endpoint, token, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn strip_connection_ids(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("connectionId");
            for child in map.values_mut() {
                strip_connection_ids(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                strip_connection_ids(item);
            }
        }
        _ => {}
    }
}

fn order_documents(yaml: &str) -> (String, String) {
    let mut first = None;
    let mut second = None;
    for document in yaml.split("\n---\n") {
        if !document.contains("name: order") {
            continue;
        }
        if document.contains("version:") {
            second = Some(document.to_string());
        } else {
            first = Some(document.to_string());
        }
    }
    (
        first.expect("order version 1"),
        second.expect("order version 2"),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_cli_absent_aggregate_reapply_isolates_versions() {
    use serde_json::json;
    use trogon_atlas_proto as pb;

    let endpoint = authenticated_server().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema.yaml");
    let product = json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {"namespace": "cli-accept", "name": "product"},
        "spec": {"title": "Product", "fields": [{"name": "sku", "type": {"string": {}}}]}
    });
    std::fs::write(&path, product.to_string()).unwrap();
    authed_ok(&endpoint, WRITER, &["apply", "-f", path.to_str().unwrap()]);
    let order = json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {"namespace": "cli-accept", "name": "order"},
        "spec": {
            "title": "Order",
            "fields": [
                {"name": "customer", "repeated": false, "type": {"object": {"fields": [
                    {"name": "id", "repeated": false, "type": {"string": {}}}
                ]}}},
                {"name": "product_ref", "type": {"ref": {"ref": {"schema": "product"}}}}
            ],
            "identifiers": [{
                "property": {"segments": [
                    {"name": "customer", "repeated": false},
                    {"name": "id", "repeated": false}
                ]},
                "role": "BUSINESS_IDENTIFIER_ROLE_PRIMARY"
            }],
            "propertyRelationships": [{
                "source": {"segments": [{"name": "product_ref", "repeated": false}]},
                "targetEntity": "product",
                "label": {"value": "hasMany"}
            }]
        }
    });
    std::fs::write(&path, order.to_string()).unwrap();
    authed_ok(&endpoint, WRITER, &["apply", "-f", path.to_str().unwrap()]);
    let second = json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {"namespace": "cli-accept", "name": "order", "version": "2"},
        "spec": {"title": "Order v2", "aggregateRoot": true, "fields": [{"name": "id", "type": {"string": {}}}]}
    });
    std::fs::write(&path, second.to_string()).unwrap();
    authed_ok(&endpoint, WRITER, &["apply", "-f", path.to_str().unwrap()]);

    let v1_address = "schema/cli-accept/order@1";
    let v2_address = "schema/cli-accept/order@2";
    let v1 = authed_ok(&endpoint, READER, &["get", v1_address]);
    let v1_manifest = &v1["data"]["manifest"];
    assert!(
        v1_manifest["spec"].get("aggregateRoot").is_none(),
        "{v1_manifest}"
    );
    let v2 = authed_ok(&endpoint, READER, &["get", v2_address]);
    assert_eq!(v2["data"]["manifest"]["spec"]["aggregateRoot"], true);
    let described = authed_ok(&endpoint, READER, &["describe", v1_address]);
    let relationship_id = v1_manifest["spec"]["propertyRelationships"][0]["connectionId"]["value"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        described.to_string().contains(&relationship_id),
        "{described}"
    );

    let mut native = trogon_atlas_client::client::connect(&endpoint, Some(WRITER), 30)
        .await
        .unwrap();
    let read = async |client: &mut trogon_atlas_client::client::Client, version: u64| {
        client
            .get_entity(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(pb::Id {
                    namespace: "cli-accept".into(),
                    slug: "order".into(),
                    version,
                }),
            })
            .await
            .unwrap()
            .into_inner()
    };
    let before_v1 = read(&mut native, 1).await;
    let before_v2 = read(&mut native, 2).await;
    let uid_v1 = before_v1
        .entity
        .as_ref()
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone();
    let uid_v2 = before_v2
        .entity
        .as_ref()
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone();
    assert_ne!(uid_v1, uid_v2);
    let pb::entity::Kind::Schema(schema) =
        before_v1.entity.as_ref().unwrap().kind.as_ref().unwrap()
    else {
        panic!("schema");
    };
    assert_eq!(schema.aggregate_root, None);
    let pb::entity::Kind::Schema(schema) =
        before_v2.entity.as_ref().unwrap().kind.as_ref().unwrap()
    else {
        panic!("schema");
    };
    assert_eq!(schema.aggregate_root, Some(true));

    let mut edited = v1_manifest.clone();
    edited["spec"]["title"] = json!("Order edited");
    edited["spec"]["propertyRelationships"][0]["label"] = json!({"value": "belongsTo"});
    assert!(edited["spec"].get("aggregateRoot").is_none());
    std::fs::write(&path, edited.to_string()).unwrap();
    authed_ok(&endpoint, WRITER, &["apply", "-f", path.to_str().unwrap()]);
    let edited_v1 = read(&mut native, 1).await;
    assert_ne!(edited_v1.etag, before_v1.etag);
    assert_eq!(
        edited_v1
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid,
        uid_v1
    );
    let pb::entity::Kind::Schema(schema) =
        edited_v1.entity.as_ref().unwrap().kind.as_ref().unwrap()
    else {
        panic!("schema");
    };
    assert_eq!(schema.aggregate_root, None);
    assert_eq!(schema.title, "Order edited");
    assert_eq!(
        schema.property_relationships[0]
            .label
            .as_ref()
            .unwrap()
            .value,
        "belongsTo"
    );
    assert_eq!(
        schema.property_relationships[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        relationship_id
    );
    let unchanged_v2 = read(&mut native, 2).await;
    assert_eq!(unchanged_v2, before_v2);

    let export = dir.path().join("export");
    authed_ok(
        &endpoint,
        WRITER,
        &[
            "export",
            "--namespace",
            "cli-accept",
            "--output",
            export.to_str().unwrap(),
        ],
    );
    let yaml = std::fs::read_to_string(export.join("schema.yaml")).unwrap();
    let (first, second) = order_documents(&yaml);
    assert!(!first.contains("aggregateRoot"), "{first}");
    assert!(first.contains("belongsTo"), "{first}");
    assert!(second.contains("aggregateRoot: true"), "{second}");
    authed_ok(&endpoint, WRITER, &["diff", "-f", export.to_str().unwrap()]);
    authed_ok(
        &endpoint,
        WRITER,
        &["apply", "-f", export.to_str().unwrap()],
    );
    assert_eq!(read(&mut native, 1).await, edited_v1);
    assert_eq!(read(&mut native, 2).await, before_v2);

    let mut stripped =
        authed_ok(&endpoint, WRITER, &["get", v1_address])["data"]["manifest"].clone();
    strip_connection_ids(&mut stripped);
    assert!(stripped["spec"].get("aggregateRoot").is_none());
    std::fs::write(&path, stripped.to_string()).unwrap();
    authed_ok(&endpoint, WRITER, &["apply", "-f", path.to_str().unwrap()]);
    assert_eq!(read(&mut native, 1).await, edited_v1);

    let mut denied_manifest = stripped;
    denied_manifest["spec"]["title"] = json!("Denied");
    std::fs::write(&path, denied_manifest.to_string()).unwrap();
    let denied = authed(&endpoint, READER, &["apply", "-f", path.to_str().unwrap()]);
    assert!(
        !denied.status.success(),
        "{}",
        String::from_utf8_lossy(&denied.stdout)
    );
    assert_eq!(read(&mut native, 1).await, edited_v1);
    assert_eq!(read(&mut native, 2).await, before_v2);
}
