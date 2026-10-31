#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

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

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn cli(endpoint: &str, args: &[&str]) -> std::process::Output {
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
    result
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn id(namespace: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version,
    }
}

async fn stored(
    endpoint: &str,
    kind: pb::EntityKind,
    namespace: &str,
    slug: &str,
    version: u64,
) -> pb::Entity {
    let mut client = trogon_atlas_client::client::connect(endpoint, None, 30)
        .await
        .unwrap();
    client
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(id(namespace, slug, version)),
        })
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_authoring_inspect_diff_export_and_reapply_keep_native_declarations() {
    let endpoint = start_server().await;
    let ns = unique("assoc");
    let directory = tempfile::tempdir().unwrap();
    let anchors = directory.path().join("anchors.yaml");
    std::fs::write(
        &anchors,
        format!(
            r#"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {{namespace: {ns}, name: placed}}
spec: {{title: Placed}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {{namespace: {ns}, name: wide, version: "18446744073709551615"}}
spec: {{title: Wide}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: {{namespace: {ns}, name: place}}
spec: {{title: Place}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Schema
metadata: {{namespace: {ns}, name: shape}}
spec: {{title: Shape}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModel
metadata: {{namespace: {ns}, name: receipt, version: 2}}
spec: {{title: Receipt}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Processor
metadata: {{namespace: {ns}, name: worker}}
spec: {{title: Worker}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Component
metadata: {{namespace: {ns}, name: checkout}}
spec: {{title: Checkout}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ExternalSystem
metadata: {{namespace: {ns}, name: partner}}
spec: {{title: Partner}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DataStore
metadata: {{namespace: {ns}, name: cache}}
spec:
  title: Cache
  storeClass: {{known: KNOWN_CACHE}}
  authoritative: false
  attachments:
    - readModel: receipt@2
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: AutomationSlice
metadata: {{namespace: {ns}, name: observe}}
spec:
  title: Observe
  sourceReadModels: [receipt@2]
  processor: worker
"#
        ),
    )
    .unwrap();
    cli(&endpoint, &["apply", "-f", anchors.to_str().unwrap()]);
    let wide = stored(&endpoint, pb::EntityKind::Event, &ns, "wide", u64::MAX).await;
    assert_eq!(
        trogon_atlas_core::refs::entity_id(&wide).unwrap().version,
        u64::MAX
    );
    let owner = stored(
        &endpoint,
        pb::EntityKind::AutomationSlice,
        &ns,
        "observe",
        1,
    )
    .await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = &owner.kind else {
        panic!("automation");
    };
    let observation = serde_json::json!({
        "owner": {"kind": "ENTITY_KIND_AUTOMATION_SLICE", "id": {"namespace": ns, "slug": "observe", "version": "1"}},
        "ownerUid": owner.system.as_ref().unwrap().uid,
        "id": {"value": slice.source_read_models[0].connection_id.as_ref().unwrap().value}
    });
    let channel = |name: &str| {
        serde_json::json!({
            "apiVersion": "eventmodel.atlas.trogonstack.com/v1alpha1",
            "kind": "Channel",
            "metadata": {"namespace": ns, "name": name},
            "spec": {
                "title": name,
                "protocols": [{"value": "nats"}],
                "address": {
                    "template": "orders.{topic}.{note}",
                    "parameters": [
                        {"name": {"value": "topic"}},
                        {"name": {"value": "note"}}
                    ]
                },
                "bindings": [{
                    "observation": observation,
                    "parameterValues": {"topic": "orders", "note": "kept"}
                }]
            }
        })
    };
    let channels = directory.path().join("channels.yaml");
    std::fs::write(
        &channels,
        format!("{}\n---\n{}\n", channel("orders"), channel("backup")),
    )
    .unwrap();
    cli(&endpoint, &["apply", "-f", channels.to_str().unwrap()]);

    let content = "{\"type\":\"object\"}\r\n";
    let digest = trogon_atlas_core::specification::content_digest(content).sha256;
    let environment = "a".repeat(64);
    let specification = serde_json::json!({
        "apiVersion": "eventmodel.atlas.trogonstack.com/v1alpha1",
        "kind": "Specification",
        "metadata": {"namespace": ns, "name": "contract"},
        "spec": {
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
        }
    });
    let contract = directory.path().join("contract.yaml");
    std::fs::write(&contract, specification.to_string()).unwrap();
    cli(&endpoint, &["apply", "-f", contract.to_str().unwrap()]);
    let stored_spec = stored(&endpoint, pb::EntityKind::Specification, &ns, "contract", 1).await;
    let spec_uid = stored_spec.system.as_ref().unwrap().uid.clone();
    let spec_etag = {
        let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
            .await
            .unwrap();
        client
            .get_entity(pb::GetEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: Some(id(&ns, "contract", 1)),
            })
            .await
            .unwrap()
            .into_inner()
            .etag
    };
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
    assert_ne!(
        contract.message_channels[0].connection_id,
        contract.message_channels[1].connection_id
    );

    let product = serde_json::json!({
        "apiVersion": "eventmodel.atlas.trogonstack.com/v1alpha1",
        "kind": "DataProduct",
        "metadata": {"namespace": ns, "name": "offering"},
        "spec": {
            "title": "Offering",
            "outputs": [{"name": {"value": "Receipt"}, "data": {"readModel": "receipt@2"}}],
            "resourceAssociations": [
                {
                    "direction": "DATA_PORT_DIRECTION_OUTPUT",
                    "event": {"id": {"namespace": ns, "slug": "placed", "version": "1"}},
                    "contract": {"schema": {"id": {"namespace": ns, "slug": "shape", "version": "1"}}},
                    "doc": "descriptive"
                },
                {
                    "direction": "DATA_PORT_DIRECTION_OUTPUT",
                    "event": {"id": {"namespace": ns, "slug": "placed", "version": "1"}}
                },
                {
                    "direction": "DATA_PORT_DIRECTION_INPUT",
                    "command": {"id": {"namespace": ns, "slug": "place", "version": "1"}}
                },
                {
                    "direction": "DATA_PORT_DIRECTION_OUTPUT",
                    "message": {
                        "specification": {"id": {"namespace": ns, "slug": "contract", "version": "1"}},
                        "ownerUid": spec_uid,
                        "kind": "CONTRACT_MEMBER_KIND_MESSAGE",
                        "memberId": {"value": "22222222-2222-4222-8222-222222222222"}
                    }
                },
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "component": {"id": {"namespace": ns, "slug": "checkout", "version": "1"}}},
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "externalSystem": {"id": {"namespace": ns, "slug": "partner", "version": "1"}}},
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "channel": {"id": {"namespace": ns, "slug": "orders", "version": "1"}}},
                {"direction": "DATA_PORT_DIRECTION_OUTPUT", "dataStore": {"id": {"namespace": ns, "slug": "cache", "version": "1"}}}
            ]
        }
    });
    let offering = directory.path().join("offering.yaml");
    std::fs::write(&offering, product.to_string()).unwrap();
    cli(&endpoint, &["apply", "-f", offering.to_str().unwrap()]);
    let before = stored(&endpoint, pb::EntityKind::DataProduct, &ns, "offering", 1).await;
    let before_uid = before.system.as_ref().unwrap().uid.clone();
    let Some(pb::entity::Kind::DataProduct(value)) = &before.kind else {
        panic!("product");
    };
    assert_eq!(value.outputs.len(), 1);
    assert_eq!(value.resource_associations.len(), 8);
    let ids: std::collections::HashSet<_> = value
        .resource_associations
        .iter()
        .map(|association| association.connection_id.as_ref().unwrap().value.clone())
        .collect();
    assert_eq!(ids.len(), 8);
    assert!(value.resource_associations[0].contract.is_some());
    assert!(value.resource_associations[1].contract.is_none());
    let inspection = stdout(&cli(
        &endpoint,
        &[
            "inspect-data-product",
            &format!("data_product/{ns}/offering@1"),
        ],
    ));
    let inspection: serde_json::Value = serde_json::from_str(&inspection).unwrap();
    assert_eq!(inspection["ports"].as_array().unwrap().len(), 1);

    let export = directory.path().join("export");
    cli(
        &endpoint,
        &[
            "export",
            "--namespace",
            &ns,
            "--output",
            export.to_str().unwrap(),
        ],
    );
    let diff = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(&endpoint)
        .args(["diff", "-f", export.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        diff.status.success(),
        "diff {}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let reapplied = stdout(&cli(&endpoint, &["apply", "-f", export.to_str().unwrap()]));
    assert!(!reapplied.contains("created"), "{reapplied}");
    assert!(!reapplied.contains("updated"), "{reapplied}");
    let after = stored(&endpoint, pb::EntityKind::DataProduct, &ns, "offering", 1).await;
    assert_eq!(after.system.as_ref().unwrap().uid, before_uid);
    assert_eq!(after.kind, before.kind);
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let after_spec = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(id(&ns, "contract", 1)),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after_spec.etag, spec_etag);
    assert_eq!(after_spec.entity.unwrap().kind, stored_spec.kind);
}
