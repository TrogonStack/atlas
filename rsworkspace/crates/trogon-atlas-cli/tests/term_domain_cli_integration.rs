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
kind: Domain
metadata: {namespace: vocabulary, name: sales}
spec: {title: Sales}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Term
metadata: {namespace: vocabulary, name: order}
spec:
  title: Order
  owningDomain: {id: {namespace: vocabulary, slug: sales, version: '1'}}
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn term_domain_cli_roundtrip_and_paged_query() {
    let endpoint = start_server().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("terms.yaml");
    std::fs::write(&path, MANIFEST).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let first = cli(&endpoint, &["get", "term/vocabulary/order@1"]);
    assert!(first.contains("owningDomain"));
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    assert_eq!(first, cli(&endpoint, &["get", "term/vocabulary/order@1"]));
    let query =
        r#"{"domain":{"id":{"namespace":"vocabulary","slug":"sales","version":"1"}},"pageSize":1}"#;
    let listed = cli(
        &endpoint,
        &["list-term-domain-declarations", "--request", query],
    );
    assert!(listed.contains("declarations"));
    assert!(listed.contains("owningDomain") || listed.contains("domain"));
}
