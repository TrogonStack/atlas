#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
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
async fn architectural_cli_authoring_export_reapply_and_exact_subject_queries() {
    use serde_json::json;
    use trogon_atlas_proto as pb;
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("architecture.yaml");
    std::fs::write(
        &path,
        r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ExternalSystem
metadata: {namespace: cli-architecture, name: provider, version: 9007199254740993}
spec: {title: Provider}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Component
metadata: {namespace: cli-architecture, name: checkout}
spec:
  title: Checkout
  relationships:
    - target: {externalSystem: provider@9007199254740993}
      label: {value: submits orders}
      doc: {value: Architectural intent}
    - target: {externalSystem: provider@9007199254740993}
      label: {value: submits orders}
      doc: {value: Architectural intent}
",
    )
    .unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let before = cli(&endpoint, &["get", "component/cli-architecture/checkout@1"]);
    let entity = trogon_atlas_client::manifest::parse_str(&before, "read.yaml")
        .unwrap()
        .remove(0)
        .entity;
    let relations = trogon_atlas_core::architectural_relationships::relationships(&entity);
    let ids: Vec<_> = relations
        .iter()
        .map(|item| item.connection_id.as_ref().unwrap().value.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    let mut native =
        pb::event_model_service_client::EventModelServiceClient::connect(endpoint.clone())
            .await
            .unwrap();
    let response = native
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(pb::Id {
                namespace: "cli-architecture".into(),
                slug: "checkout".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    let owner_uid = response
        .entity
        .as_ref()
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone();
    let subject = |value: &str| json!({"connection":{"owner":{"kind":"ENTITY_KIND_COMPONENT","id":{"namespace":"cli-architecture","slug":"checkout","version":"1"}},"ownerUid":owner_uid,"id":{"value":value}}});
    let team = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Team","metadata":{"namespace":"cli-architecture","name":"team"},"spec":{"title":{"value":"Operators"},"responsibilities":[{"kind":"RESPONSIBILITY_KIND_CONTACT","subject":subject(&ids[0])}]}});
    let team_path = dir.path().join("team.yaml");
    std::fs::write(&team_path, team.to_string()).unwrap();
    cli(&endpoint, &["apply", "-f", team_path.to_str().unwrap()]);
    for (index, value) in ids.iter().enumerate() {
        let result: serde_json::Value = serde_json::from_str(&cli(
            &endpoint,
            &[
                "list-subject-assignments",
                "--request",
                &json!({"subject":subject(value)}).to_string(),
            ],
        ))
        .unwrap();
        assert_eq!(
            result["assignments"].as_array().map_or(0, Vec::len),
            usize::from(index == 0),
            "{result}"
        );
        assert!(result["scope"].is_object());
    }
    for request in [
        json!({"subject":{"connection":{"ownerUid":false}}}),
        json!({"subject":{"connection":{"path":"relationships[0]"}}}),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
            .args([
                "--endpoint",
                &endpoint,
                "list-subject-assignments",
                "--request",
                &request.to_string(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
    let described = cli(
        &endpoint,
        &["describe", "component/cli-architecture/checkout@1"],
    );
    for value in &ids {
        assert!(described.contains(value));
    }
    let export = dir.path().join("export");
    cli(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-architecture",
            "--output",
            export.to_str().unwrap(),
        ],
    );
    cli(&endpoint, &["diff", "-f", export.to_str().unwrap()]);
    cli(&endpoint, &["apply", "-f", export.to_str().unwrap()]);
    let after = native
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(pb::Id {
                namespace: "cli-architecture".into(),
                slug: "checkout".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after, response);
    assert_eq!(
        before,
        cli(&endpoint, &["get", "component/cli-architecture/checkout@1"])
    );
}
