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
    assert!(!uid.is_empty());

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
