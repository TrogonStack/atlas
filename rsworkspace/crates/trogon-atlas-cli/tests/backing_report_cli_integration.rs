#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server(cap: Option<usize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    if let Some(cap) = cap {
        service = service.with_snapshot_cap(cap);
    }
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn cli(endpoint: &str, args: &[&str]) -> (Option<i32>, String) {
    let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .unwrap();
    (
        result.status.code(),
        String::from_utf8(result.stdout).unwrap(),
    )
}

const MANIFEST: &str = r#"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {namespace: backing-cli, name: noted}
spec: {title: Noted}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {namespace: backing-cli, name: bare}
spec: {title: Bare}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: backing-cli, name: note}
spec:
  title: {value: Note}
  subjects: [{subject: {entity: {kind: event, id: noted}}}]
  retained:
    mediaType: {value: text/markdown}
    content: "Retained notes\n"
    digest: {sha256: 125a1e44e8f47bf4e244de8f09f31442749f57260dec8319d252e2992a3b16f1}
"#;

fn apply(endpoint: &str) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("backing.yaml");
    std::fs::write(&path, MANIFEST).unwrap();
    let (code, _) = cli(endpoint, &["apply", "-f", path.to_str().unwrap()]);
    assert_eq!(code, Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backing_report_exits_zero_when_every_subject_is_settled() {
    let endpoint = start_server(None).await;
    apply(&endpoint);
    let (code, text) = cli(
        &endpoint,
        &[
            "backing-report",
            "--namespace",
            "backing-cli",
            "--unbacked-findings",
        ],
    );
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("2 subjects, 1 backed, 1 unbacked, 0 unknown, unbacked rate 50.0%"),
        "{text}"
    );
    assert!(text.contains("ENTITY_UNBACKED"), "{text}");

    let (code, json) = cli(
        &endpoint,
        &[
            "--format",
            "json",
            "backing-report",
            "--namespace",
            "backing-cli",
        ],
    );
    assert_eq!(code, Some(0), "{json}");
    let outcome: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    assert_eq!(outcome["ok"], serde_json::json!(true));
    assert_eq!(outcome["data"]["totals"]["backed"], serde_json::json!(1));
    assert_eq!(
        outcome["data"]["totals"]["unbackedRate"],
        serde_json::json!(0.5)
    );
    assert_eq!(outcome["data"]["snapshotId"].as_str().unwrap().len(), 64);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backing_report_exits_incomplete_on_a_truncated_snapshot() {
    let endpoint = start_server(Some(1)).await;
    apply(&endpoint);
    let (code, text) = cli(&endpoint, &["backing-report"]);
    assert_eq!(code, Some(11), "{text}");
    assert!(text.contains("unbacked rate not established"), "{text}");
    assert!(text.contains("incomplete: snapshot_truncated"), "{text}");

    let (code, json) = cli(&endpoint, &["--format", "json", "backing-report"]);
    assert_eq!(code, Some(11), "{json}");
    let outcome: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    assert_eq!(
        outcome["data"]["snapshotTruncated"],
        serde_json::json!(true)
    );
    assert!(outcome["data"]["totals"].get("unbackedRate").is_none());
}
