#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

#[test]
fn cli_exposes_exact_human_interaction_inspection() {
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(["inspect-human-interaction", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("--request"));
}

#[path = "../../trogon-atlas-client/tests/support/human_interaction_fixture.rs"]
mod fixture;

use std::sync::Arc;

use fixture::{ADMIN, BRANCH, HIDDEN, OWNER, READER, VERSION, WRITER};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_client::client::{connect, connect_with_options, ConnectOptions};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

async fn server() -> (String, String, tokio::task::JoinHandle<()>) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals":{
            "admin":{"role":"admin","tokens":[ADMIN]},
            "writer":{"role":"writer","kind":"agent","parent":OWNER,"tokens":[WRITER]},
            "reader":{"role":"reader","kind":"agent","parent":OWNER,"tokens":[READER]},
            "hidden":{"role":"reader","kind":"agent","parent":"outside","tokens":[HIDDEN]}
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
        EventModelServiceServer::new(service),
        BearerAuth::new(registry),
    );
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(secured)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let endpoint = format!("http://{address}");
    let mut client = connect(&endpoint, Some(ADMIN), 30).await.unwrap();
    let namespace = client
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "human-cli".into(),
            parent: OWNER.into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    client
        .create_branch(pb::CreateBranchRequest {
            name: BRANCH.into(),
            doc: "Human inspection".into(),
        })
        .await
        .unwrap();
    (endpoint, namespace, task)
}

fn command(endpoint: &str, token: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            endpoint,
            "--auth-token",
            token,
            "--branch",
            BRANCH,
            "--format",
            "json",
        ])
        .args(args)
        .output()
        .unwrap()
}

fn succeed(endpoint: &str, token: &str, args: &[&str]) -> Value {
    let output = command(endpoint, token, args);
    assert!(
        output.status.success(),
        "{args:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_cli_authors_and_inspects_exact_hosting_through_export_reapply() {
    let (endpoint, namespace, task) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let manifest = directory.path().join("hosting.yaml");
    std::fs::write(&manifest, fixture::manifests(&namespace)).unwrap();
    let path = manifest.to_str().unwrap();
    succeed(&endpoint, WRITER, &["apply", "-f", path, "--dry-run"]);
    succeed(&endpoint, WRITER, &["apply", "-f", path]);
    let mut writer = connect_with_options(
        &endpoint,
        Some(WRITER),
        30,
        ConnectOptions {
            branch: Some(BRANCH.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (issuance, display, before) = fixture::selections(&mut writer, &namespace).await;
    let hosted = before.entity.as_ref().unwrap();
    let Some(pb::entity::Kind::Component(component)) = &hosted.kind else {
        panic!("Component")
    };
    assert_ne!(
        component.ui_hostings[0].connection_id,
        component.ui_hostings[1].connection_id
    );
    assert!(component.ui_hostings[0].doc.is_none());
    assert_eq!(
        component.ui_hostings[1].doc.as_ref().unwrap().value,
        "<script>inert</script>"
    );
    for request in [&issuance, &display] {
        let response = succeed(
            &endpoint,
            READER,
            &[
                "inspect-human-interaction",
                "--request",
                &request.to_string(),
            ],
        );
        fixture::assert_checked(&response, request);
    }
    let mut stale = issuance.clone();
    stale["hosting"]["ownerUid"] = json!("11111111-1111-4111-8111-111111111111");
    let unresolved = succeed(
        &endpoint,
        READER,
        &["inspect-human-interaction", "--request", &stale.to_string()],
    );
    assert_eq!(unresolved["state"], "CATALOG_RESOLUTION_STATE_UNRESOLVED");
    assert_eq!(unresolved["request"], stale);
    assert!(unresolved
        .get("edges")
        .is_none_or(|edges| edges.as_array().unwrap().is_empty()));
    let document = fixture::manifest(
        &namespace,
        "Document",
        "hosting-note",
        &json!({
            "title":{"value":"Hosting assertion"},"link":{"value":"https://example.invalid/hosting"},
            "subjects":[{"subject":{"connection":issuance["hosting"]}}]
        }),
    );
    std::fs::write(&manifest, document).unwrap();
    succeed(&endpoint, WRITER, &["apply", "-f", path]);
    let subjects = succeed(
        &endpoint,
        READER,
        &[
            "list-subject-documents",
            "--request",
            &json!({"subject":{"connection":issuance["hosting"]}}).to_string(),
        ],
    );
    assert_eq!(subjects["declarations"].as_array().unwrap().len(), 1);
    assert_eq!(
        subjects["declarations"][0]["subject"]["subject"]["connection"],
        issuance["hosting"]
    );
    let key = format!("component/{namespace}/portal@{VERSION}");
    let described = succeed(&endpoint, READER, &["describe", &key]);
    let outgoing = described["outgoing"].as_array().unwrap();
    assert_eq!(outgoing.len(), 2);
    for (index, row) in outgoing.iter().enumerate() {
        assert_eq!(
            row["provenance"]["fromUid"],
            hosted.system.as_ref().unwrap().uid
        );
        assert_eq!(
            row["provenance"]["connectionId"]["value"],
            component.ui_hostings[index]
                .connection_id
                .as_ref()
                .unwrap()
                .value
        );
    }
    let exported = directory.path().join("export");
    succeed(
        &endpoint,
        READER,
        &[
            "export",
            "--namespace",
            &namespace,
            "-o",
            exported.to_str().unwrap(),
        ],
    );
    for entry in std::fs::read_dir(&exported).unwrap() {
        let export = entry.unwrap().path();
        let path = export.to_str().unwrap();
        succeed(&endpoint, READER, &["diff", "-f", path]);
        succeed(&endpoint, WRITER, &["apply", "-f", path]);
    }
    assert_eq!(
        fixture::get(&mut writer, &namespace, pb::EntityKind::Component, "portal").await,
        before
    );
    std::fs::write(
        &manifest,
        fixture::manifest(
            &namespace,
            "Component",
            "reader-write",
            &json!({"title":"Reader cannot author"}),
        ),
    )
    .unwrap();
    let denied = command(&endpoint, READER, &["apply", "-f", path]);
    assert!(!denied.status.success());
    let hidden = command(
        &endpoint,
        HIDDEN,
        &[
            "inspect-human-interaction",
            "--request",
            &issuance.to_string(),
        ],
    );
    assert!(!hidden.status.success());
    let hidden_body = String::from_utf8_lossy(&hidden.stdout);
    assert!(!hidden_body.contains(&hosted.system.as_ref().unwrap().uid));
    task.abort();
}
