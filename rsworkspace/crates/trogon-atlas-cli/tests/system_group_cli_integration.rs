#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
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

fn run(endpoint: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .unwrap()
}

fn cli(endpoint: &str, args: &[&str]) -> String {
    let result = run(endpoint, args);
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn group_cli_authoring_export_reapply_and_exact_subject_queries() {
    use serde_json::json;
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("groups.yaml");
    std::fs::write(
        &path,
        r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Component
metadata: {namespace: cli-groups, name: worker}
spec: {title: Worker}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: SystemGroup
metadata: {namespace: cli-groups, name: platform}
spec:
  title: Platform
  scope: SYSTEM_GROUP_SCOPE_INTERNAL
  members:
    - member: {component: worker@1}
      doc: {value: Curated}
    - member: {component: worker@1}
      doc: {value: Curated}
",
    )
    .unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let before = cli(&endpoint, &["get", "system_group/cli-groups/platform@1"]);
    let entity = trogon_atlas_client::manifest::parse_str(&before, "read.yaml")
        .unwrap()
        .remove(0)
        .entity;
    let Some(pb::entity::Kind::SystemGroup(group)) = &entity.kind else {
        panic!("expected system group");
    };
    let ids: Vec<_> = group
        .members
        .iter()
        .map(|item| item.connection_id.as_ref().unwrap().value.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);

    let mut native =
        pb::event_model_service_client::EventModelServiceClient::connect(endpoint.clone())
            .await
            .unwrap();
    let request = pb::GetEntityRequest {
        kind: pb::EntityKind::SystemGroup as i32,
        id: Some(pb::Id {
            namespace: "cli-groups".into(),
            slug: "platform".into(),
            version: 1,
        }),
    };
    let response = native
        .get_entity(request.clone())
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
    let subject = |value: &str| json!({"connection":{"owner":{"kind":"ENTITY_KIND_SYSTEM_GROUP","id":{"namespace":"cli-groups","slug":"platform","version":"1"}},"ownerUid":owner_uid,"id":{"value":value}}});
    let team = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Team","metadata":{"namespace":"cli-groups","name":"team"},"spec":{"title":{"value":"Operators"},"responsibilities":[{"kind":"RESPONSIBILITY_KIND_CONTACT","subject":subject(&ids[0])}]}});
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
    }
    let described = cli(
        &endpoint,
        &["describe", "system_group/cli-groups/platform@1"],
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
            "cli-groups",
            "--output",
            export.to_str().unwrap(),
        ],
    );
    cli(&endpoint, &["diff", "-f", export.to_str().unwrap()]);
    cli(&endpoint, &["apply", "-f", export.to_str().unwrap()]);
    assert_eq!(
        native.get_entity(request).await.unwrap().into_inner(),
        response
    );
    assert_eq!(
        before,
        cli(&endpoint, &["get", "system_group/cli-groups/platform@1"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn group_cli_refuses_self_membership_and_unresolved_targets_without_writing() {
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, member: &str| {
        let path = dir.path().join(name);
        std::fs::write(
            &path,
            format!(
                "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: SystemGroup\nmetadata: {{namespace: cli-refused, name: loop}}\nspec:\n  title: Loop\n  members:\n    - member: {{{member}}}\n"
            ),
        )
        .unwrap();
        path
    };
    for (name, member) in [
        ("self.yaml", "systemGroup: loop@1"),
        ("missing.yaml", "component: ghost@1"),
        ("pending.yaml", "designGraph: graph@1"),
    ] {
        let path = write(name, member);
        let output = run(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
        assert!(!output.status.success(), "{name}");
    }
    let output = run(&endpoint, &["get", "system_group/cli-refused/loop@1"]);
    assert!(!output.status.success());
}
