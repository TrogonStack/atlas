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
async fn responsibility_cli_native_authoring_queries_and_reapply_preserve_ids() {
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("directory.yaml");
    std::fs::write(
        &path,
        r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Person
metadata: {namespace: cli-directory, name: person}
spec:
  title: {value: Person}
  contacts: [{email: {value: catalog@example.test}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Team
metadata: {namespace: cli-directory, name: team}
spec:
  title: {value: Team}
  members: [{person: person}]
  responsibilities:
    - kind: RESPONSIBILITY_KIND_OWNER
      subject: {entity: {kind: person, id: person}}
",
    )
    .unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let before = cli(&endpoint, &["get", "team/cli-directory/team@1"]);
    let parsed = trogon_atlas_client::manifest::parse_str(&before, "team.yaml").unwrap();
    let Some(trogon_atlas_proto::entity::Kind::Team(team)) = &parsed[0].entity.kind else {
        panic!("team missing")
    };
    assert_ne!(team.members[0].connection_id.as_ref().unwrap().value, "");
    assert_ne!(
        team.responsibilities[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        ""
    );
    let person =
        serde_json::json!({"id":{"namespace":"cli-directory","slug":"person","version":"1"}});
    let team_ref =
        serde_json::json!({"id":{"namespace":"cli-directory","slug":"team","version":"1"}});
    for (command, request, field) in [
        (
            "list-subject-assignments",
            serde_json::json!({"subject":{"entity":{"kind":"ENTITY_KIND_PERSON","id":person["id"]}}}),
            "assignments",
        ),
        (
            "list-assignee-responsibilities",
            serde_json::json!({"assignee":{"team":team_ref}}),
            "assignments",
        ),
        (
            "list-team-members",
            serde_json::json!({"team":team_ref}),
            "memberships",
        ),
        (
            "list-person-teams",
            serde_json::json!({"person":person}),
            "memberships",
        ),
        (
            "get-team-contacts",
            serde_json::json!({"team":team_ref,"includeMembers":true}),
            "memberContacts",
        ),
    ] {
        let result: serde_json::Value = serde_json::from_str(&cli(
            &endpoint,
            &[command, "--request", &request.to_string()],
        ))
        .unwrap();
        assert_eq!(
            result[field].as_array().unwrap().len(),
            1,
            "{command}: {result}"
        );
        assert!(result["scope"].is_object(), "{result}");
    }
    let export = dir.path().join("export");
    cli(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-directory",
            "--output",
            export.to_str().unwrap(),
        ],
    );
    cli(&endpoint, &["apply", "-f", export.to_str().unwrap()]);
    assert_eq!(
        before,
        cli(&endpoint, &["get", "team/cli-directory/team@1"])
    );
    let member_id = &team.members[0].connection_id.as_ref().unwrap().value;
    let described = cli(&endpoint, &["describe", "team/cli-directory/team@1"]);
    assert!(described.contains("Title:  Team"));
    assert!(described.contains(member_id));
    assert!(described.contains(
        &team.responsibilities[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value
    ));
    for invalid_id in ["not-an-id", "11111111-1111-4111-8111-111111111111"] {
        let invalid_path = dir.path().join("invalid.yaml");
        std::fs::write(&invalid_path, before.replace(member_id, invalid_id)).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
            .args([
                "--endpoint",
                &endpoint,
                "--format",
                "json",
                "apply",
                "-f",
                invalid_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "unexpectedly accepted {invalid_id}"
        );
        assert_eq!(
            before,
            cli(&endpoint, &["get", "team/cli-directory/team@1"])
        );
    }
    for request in [
        "{",
        r#"{"unknown":true}"#,
        r#"{"team":{"id":{"namespace":"cli-directory","slug":"team","version":"0"}}}"#,
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
            .args([
                "--endpoint",
                &endpoint,
                "list-team-members",
                "--request",
                request,
            ])
            .output()
            .unwrap();
        assert!(!result.status.success());
    }
}
