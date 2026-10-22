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

const MANIFEST: &str = r#"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {namespace: catalog-surfaces, name: receipt}
spec: {title: Receipt}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: catalog-surfaces, name: note}
spec:
  title: {value: Retained note}
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  retained:
    mediaType: {value: text/markdown}
    content: "Retained notes\n"
    digest: {sha256: 125a1e44e8f47bf4e244de8f09f31442749f57260dec8319d252e2992a3b16f1}
  anchors: [{fragment: {value: Delivery}, label: {value: Delivery}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Decision
metadata: {namespace: catalog-surfaces, name: choice}
spec:
  title: {value: Delivery choice}
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  status: DECISION_STATUS_PROPOSED
  context: {value: Delivery question}
  rationale: {value: Pending adoption}
  options: [{title: {value: Retain}, description: {value: Retain note}}]
  documents: [{document: {document: note}}]
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_catalog_cli_apply_get_query_and_unchanged_reapply() {
    let endpoint = start_server().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog.yaml");
    std::fs::write(&path, MANIFEST).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let before = cli(&endpoint, &["get", "document/catalog-surfaces/note@1"]);
    let document = trogon_atlas_client::manifest::parse_str(&before, "export.yaml").unwrap();
    let Some(trogon_atlas_proto::entity::Kind::Document(note)) = &document[0].entity.kind else {
        panic!("document missing");
    };
    assert_ne!(note.anchors[0].anchor_id.as_ref().unwrap().value, "");
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    assert_eq!(
        before,
        cli(&endpoint, &["get", "document/catalog-surfaces/note@1"])
    );
    let id = |slug| serde_json::json!({"namespace":"catalog-surfaces","slug":slug,"version":"1"});
    let subject = serde_json::json!({"entity":{"kind":"ENTITY_KIND_EVENT","id":id("receipt")}});
    for (command, request) in [
        (
            "list-subject-decisions",
            serde_json::json!({"subject":subject}),
        ),
        (
            "list-subject-documents",
            serde_json::json!({"subject":subject}),
        ),
        (
            "get-decision-detail",
            serde_json::json!({"decision":{"id":id("choice")},"followReplacement":false}),
        ),
        (
            "get-document-detail",
            serde_json::json!({"document":{"document":{"id":id("note")}}}),
        ),
        (
            "list-incoming-decision-declarations",
            serde_json::json!({"decision":{"id":id("choice")}}),
        ),
        (
            "list-incoming-document-declarations",
            serde_json::json!({"document":{"document":{"id":id("note")}}}),
        ),
    ] {
        let response = cli(&endpoint, &[command, "--request", &request.to_string()]);
        assert!(response.contains("scope"), "{command}: {response}");
    }
}
#[test]
fn malformed_native_catalog_query_fails_before_connecting() {
    for (command, request) in [
        ("get-document-detail", "{not json"),
        ("get-decision-detail", r#"{"followReplacement":"false"}"#),
        ("list-subject-decisions", r#"{"status":"TYPO"}"#),
        (
            "get-document-detail",
            r#"{"document":{"document":{},"anchor":{}}}"#,
        ),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
            .args([
                "--endpoint",
                "http://127.0.0.1:1",
                command,
                "--request",
                request,
            ])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(5));
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(!error.contains("transport error"), "{error}");
    }
}
